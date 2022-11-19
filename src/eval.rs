use crate::call_pattern::{CallPattern, DynResponder, PatIndex};
use crate::debug;
use crate::error;
use crate::error::{MockError, MockResult};
use crate::fn_mocker::{FnMocker, PatternMatchMode};
use crate::macro_api::Evaluation;
use crate::output::{Output, Respond, SignatureError};
use crate::state::SharedState;
use crate::value_chain::ValueChain;
use crate::DynMockFn;
use crate::{FallbackMode, MockFn};

enum EvalResult<'u> {
    Responder(EvalResponder<'u>),
    Unmock,
}

struct EvalResponder<'u> {
    fn_mocker: &'u FnMocker,
    pat_index: PatIndex,
    responder: &'u DynResponder,
}

impl<'u> EvalResponder<'u> {
    fn debug_pattern(&self) -> debug::CallPatternDebug {
        self.fn_mocker.debug_pattern(self.pat_index)
    }
}

pub(crate) fn eval<'u, 'i, F: MockFn>(
    shared_state: &'u SharedState,
    inputs: F::Inputs<'i>,
) -> MockResult<Evaluation<'u, 'i, F>> {
    let dyn_ctx = DynCtx {
        mock_fn: DynMockFn::new::<F>(),
        shared_state,
        input_debugger: &|| F::debug_inputs(&inputs),
    };

    match dyn_ctx.eval_dyn(&|pattern| pattern.match_inputs::<F>(&inputs))? {
        EvalResult::Responder(eval_rsp) => match eval_rsp.responder {
            DynResponder::Cell(inner) => match inner.downcast::<F>()?.cell.try_take() {
                Some(response) => {
                    let output =
                        response_to_output::<F>(*response, &dyn_ctx.shared_state.value_chain);
                    Ok(Evaluation::Evaluated(output))
                }
                None => Err(MockError::CannotReturnValueMoreThanOnce {
                    fn_call: dyn_ctx.fn_call(),
                    pattern: eval_rsp.debug_pattern(),
                }),
            },
            DynResponder::Borrow(inner) => {
                let downcasted = inner.downcast::<F>()?;
                match try_borrow_output_from_response::<F>(&downcasted.borrowable) {
                    Ok(output) => Ok(Evaluation::Evaluated(output)),
                    Err(_) => todo!(),
                }
            }
            DynResponder::Function(inner) => {
                let response = (inner.downcast::<F>()?.func)(inputs);
                let output = response_to_output::<F>(response, &shared_state.value_chain);
                Ok(Evaluation::Evaluated(output))
            }
            DynResponder::Panic(msg) => Err(MockError::ExplicitPanic {
                fn_call: dyn_ctx.fn_call(),
                pattern: eval_rsp.debug_pattern(),
                msg: msg.clone(),
            }),
            DynResponder::Unmock => Ok(Evaluation::Skipped(inputs)),
        },
        EvalResult::Unmock => Ok(Evaluation::Skipped(inputs)),
    }
}

/// 'u = unimock instance, 's = stack
struct DynCtx<'u, 's> {
    mock_fn: DynMockFn,
    shared_state: &'u SharedState,
    input_debugger: &'s dyn Fn() -> String,
}

impl<'u, 's> DynCtx<'u, 's> {
    #[inline(never)]
    fn eval_dyn(
        &self,
        match_inputs: &dyn Fn(&CallPattern) -> MockResult<bool>,
    ) -> MockResult<EvalResult<'u>> {
        let fn_mocker = match self.shared_state.fn_mockers.get(&self.mock_fn.type_id) {
            None => match self.shared_state.fallback_mode {
                FallbackMode::Error => {
                    return Err(MockError::NoMockImplementation {
                        name: self.mock_fn.name,
                    })
                }
                FallbackMode::Unmock => return Ok(EvalResult::Unmock),
            },
            Some(fn_mocker) => fn_mocker,
        };

        match self.match_call_pattern(fn_mocker, match_inputs)? {
            Some((pat_index, pattern)) => match pattern.next_responder() {
                Some(responder) => Ok(EvalResult::Responder(EvalResponder {
                    fn_mocker,
                    pat_index,
                    responder,
                })),
                None => Err(MockError::NoOutputAvailableForCallPattern {
                    fn_call: self.fn_call(),
                    pattern: fn_mocker.debug_pattern(pat_index),
                }),
            },
            None => match self.shared_state.fallback_mode {
                FallbackMode::Error => Err(MockError::NoMatchingCallPatterns {
                    fn_call: self.fn_call(),
                }),
                FallbackMode::Unmock => Ok(EvalResult::Unmock),
            },
        }
    }

    fn match_call_pattern(
        &self,
        fn_mocker: &'u FnMocker,
        match_inputs: &dyn Fn(&CallPattern) -> MockResult<bool>,
    ) -> MockResult<Option<(PatIndex, &'u CallPattern)>> {
        match fn_mocker.pattern_match_mode {
            PatternMatchMode::InAnyOrder => fn_mocker
                .call_patterns
                .iter()
                .enumerate()
                .filter_map(|(pat_index, pattern)| match match_inputs(pattern) {
                    Ok(false) => None,
                    Ok(true) => Some(Ok((PatIndex(pat_index), pattern))),
                    Err(err) => Some(Err(err)),
                })
                .next()
                .transpose(),
            PatternMatchMode::InOrder => {
                let ordered_call_index = self.shared_state.bump_ordered_call_index();

                let (pat_index, pattern) = fn_mocker
                    .find_call_pattern_for_call_order(ordered_call_index)
                    .ok_or_else(|| MockError::CallOrderNotMatchedForMockFn {
                        fn_call: self.fn_call(),
                        actual_call_order: error::CallOrder(ordered_call_index),
                        expected: self
                            .shared_state
                            .find_ordered_expected_call_pattern_debug(ordered_call_index),
                    })?;

                if !match_inputs(pattern)? {
                    return Err(MockError::InputsNotMatchedInCallOrder {
                        fn_call: self.fn_call(),
                        actual_call_order: error::CallOrder(ordered_call_index),
                        pattern: fn_mocker.debug_pattern(pat_index),
                    });
                }

                Ok(Some((pat_index, pattern)))
            }
        }
    }

    fn fn_call(&self) -> debug::FnActualCall {
        debug::FnActualCall {
            mock_fn: self.mock_fn.clone(),
            inputs_debug: self.debug_inputs(),
        }
    }

    fn debug_inputs(&self) -> String {
        (self.input_debugger)()
    }
}

fn response_to_output<'u, F: MockFn>(
    response: <F::Response as Respond>::Type,
    value_chain: &'u ValueChain,
) -> <F::Output<'u> as Output<'u, F::Response>>::Type {
    <F::Output<'u> as Output<'u, F::Response>>::from_response(response, value_chain)
}

fn try_borrow_output_from_response<'u, F: MockFn>(
    response: &'u <F::Response as Respond>::Type,
) -> Result<<F::Output<'u> as Output<'u, F::Response>>::Type, SignatureError> {
    <F::Output<'u> as Output<'u, F::Response>>::try_borrow_response(response)
}
