use crate::call_pattern::CallPattern;
use crate::call_pattern::Responder;
use crate::error;
use crate::error::MockError;
use crate::macro_api::Evaluation;
use crate::mock_impl::{DynMockImpl, PatternMatchMode, TypedMockImpl};
use crate::{FallbackMode, MockFn, MockInputs};

use std::any::Any;
use std::borrow::Borrow;
use std::sync::atomic::AtomicUsize;

enum Eval<C> {
    Continue(C),
    Unmock,
}

pub(crate) fn eval_sized<'i, F: MockFn + 'static>(
    dyn_impl: Option<&DynMockImpl>,
    inputs: <F as MockInputs<'i>>::Inputs,
    call_index: &AtomicUsize,
    fallback_mode: FallbackMode,
) -> Result<Evaluation<'i, F::Output, F>, MockError>
where
    F::Output: Sized,
{
    match eval_responder(dyn_impl, &inputs, call_index, fallback_mode)? {
        Eval::Continue((pat_index, responder)) => match responder {
            Responder::Value(stored) => Ok(Evaluation::Evaluated(*stored.box_clone())),
            Responder::Closure(closure) => Ok(Evaluation::Evaluated(closure(inputs))),
            Responder::StaticRefClosure(_) | Responder::Borrowable(_) => {
                Err(MockError::TypeMismatchExpectedOwnedInsteadOfBorrowed {
                    name: F::NAME,
                    inputs_debug: F::debug_inputs(&inputs),
                    pat_index,
                })
            }
            Responder::Panic(msg) => Err(MockError::ExplicitPanic {
                name: F::NAME,
                inputs_debug: F::debug_inputs(&inputs),
                pat_index,
                msg: msg.clone(),
            }),
            Responder::Unmock => Ok(Evaluation::Skipped(inputs)),
        },
        Eval::Unmock => Ok(Evaluation::Skipped(inputs)),
    }
}

pub(crate) fn eval_unsized_self_borrowed<'u, 'i, F: MockFn + 'static>(
    dyn_impl: Option<&'u DynMockImpl>,
    inputs: <F as MockInputs<'i>>::Inputs,
    call_index: &AtomicUsize,
    fallback_mode: FallbackMode,
) -> Result<Evaluation<'i, &'u F::Output, F>, MockError> {
    match eval_responder::<F>(dyn_impl, &inputs, call_index, fallback_mode)? {
        Eval::Continue((pat_index, responder)) => match responder {
            Responder::Value(stored) => Ok(Evaluation::Evaluated(stored.borrow_stored())),
            Responder::Closure(_) => Err(MockError::CannotBorrowValueProducedByClosure {
                name: F::NAME,
                inputs_debug: F::debug_inputs(&inputs),
                pat_index,
            }),
            Responder::StaticRefClosure(closure) => Ok(Evaluation::Evaluated(closure(inputs))),
            Responder::Borrowable(borrowable) => {
                let borrowable: &dyn Borrow<<F as MockFn>::Output> = borrowable.as_ref();
                let borrow = borrowable.borrow();
                Ok(Evaluation::Evaluated(borrow))
            }
            Responder::Panic(msg) => Err(MockError::ExplicitPanic {
                name: F::NAME,
                inputs_debug: F::debug_inputs(&inputs),
                pat_index,
                msg: msg.clone(),
            }),
            Responder::Unmock => Ok(Evaluation::Skipped(inputs)),
        },
        Eval::Unmock => Ok(Evaluation::Skipped(inputs)),
    }
}

pub(crate) fn eval_unsized_static_ref<'i, F: MockFn + 'static>(
    dyn_impl: Option<&DynMockImpl>,
    inputs: <F as MockInputs<'i>>::Inputs,
    call_index: &AtomicUsize,
    fallback_mode: FallbackMode,
) -> Result<Evaluation<'i, &'static F::Output, F>, MockError> {
    match eval_responder::<F>(dyn_impl, &inputs, call_index, fallback_mode)? {
        Eval::Continue((pat_index, responder)) => match responder {
            Responder::Value(_) => Err(MockError::CannotBorrowValueStatically {
                name: F::NAME,
                inputs_debug: F::debug_inputs(&inputs),
                pat_index,
            }),
            Responder::Closure(_) => Err(MockError::CannotBorrowValueProducedByClosure {
                name: F::NAME,
                inputs_debug: F::debug_inputs(&inputs),
                pat_index,
            }),
            Responder::StaticRefClosure(closure) => Ok(Evaluation::Evaluated(closure(inputs))),
            Responder::Borrowable(_) => Err(MockError::CannotBorrowValueStatically {
                name: F::NAME,
                inputs_debug: F::debug_inputs(&inputs),
                pat_index,
            }),
            Responder::Panic(msg) => Err(MockError::ExplicitPanic {
                name: F::NAME,
                inputs_debug: F::debug_inputs(&inputs),
                pat_index,
                msg: msg.clone(),
            }),
            Responder::Unmock => Ok(Evaluation::Skipped(inputs)),
        },
        Eval::Unmock => Ok(Evaluation::Skipped(inputs)),
    }
}

fn eval_responder<'u, 'i, F: MockFn + 'static>(
    dyn_impl: Option<&'u DynMockImpl>,
    inputs: &<F as MockInputs<'i>>::Inputs,
    call_index: &AtomicUsize,
    fallback_mode: FallbackMode,
) -> Result<Eval<(usize, &'u Responder<F>)>, MockError> {
    match eval_type_erased_mock_impl(dyn_impl, F::NAME, fallback_mode)? {
        Eval::Continue((any, pattern_match_mode)) => {
            let typed_impl = any
                .downcast_ref::<TypedMockImpl<F>>()
                .ok_or_else(|| MockError::Downcast { name: F::NAME })?;

            match match_pattern(pattern_match_mode, typed_impl, inputs, call_index)? {
                Some((pat_index, pattern)) => match select_responder_for_call(pattern) {
                    Some(responder) => Ok(Eval::Continue((pat_index, responder))),
                    None => Err(MockError::NoOutputAvailableForCallPattern {
                        name: F::NAME,
                        inputs_debug: F::debug_inputs(inputs),
                        pat_index,
                    }),
                },
                None => match fallback_mode {
                    FallbackMode::Error => Err(MockError::NoMatchingCallPatterns {
                        name: F::NAME,
                        inputs_debug: F::debug_inputs(inputs),
                    }),
                    FallbackMode::Unmock => Ok(Eval::Unmock),
                },
            }
        }
        Eval::Unmock => Ok(Eval::Unmock),
    }
}

#[inline(never)]
fn eval_type_erased_mock_impl<'u>(
    dyn_impl: Option<&'u DynMockImpl>,
    name: &'static str,
    fallback_mode: FallbackMode,
) -> Result<Eval<(&'u dyn Any, PatternMatchMode)>, MockError> {
    match dyn_impl {
        None => match fallback_mode {
            FallbackMode::Error => Err(MockError::NoMockImplementation { name }),
            FallbackMode::Unmock => Ok(Eval::Unmock),
        },
        Some(dyn_impl) => Ok(Eval::Continue((
            dyn_impl.typed_impl.as_ref().as_any(),
            dyn_impl.pattern_match_mode,
        ))),
    }
}

fn match_pattern<'u, 'i, F: MockFn>(
    pattern_match_mode: PatternMatchMode,
    mock_impl: &'u TypedMockImpl<F>,
    inputs: &<F as MockInputs<'i>>::Inputs,
    call_index: &AtomicUsize,
) -> Result<Option<(usize, &'u CallPattern<F>)>, MockError> {
    match pattern_match_mode {
        PatternMatchMode::InOrder => {
            // increase call index here, because stubs should not influence it:
            let current_call_index = call_index.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

            let (pat_index, pattern_by_call_index) = mock_impl
                .patterns()
                .iter()
                .enumerate()
                .find(|(_, pattern)| {
                    pattern
                        .non_generic
                        .matches_global_call_index(current_call_index)
                })
                .ok_or_else(|| MockError::CallOrderNotMatchedForMockFn {
                    name: F::NAME,
                    inputs_debug: F::debug_inputs(inputs),
                    actual_call_order: error::CallOrder(current_call_index),
                    expected_ranges: mock_impl
                        .patterns()
                        .iter()
                        .map(|pattern| pattern.non_generic.expected_range())
                        .collect(),
                })?;

            if !(pattern_by_call_index.input_matcher)(inputs) {
                return Err(MockError::InputsNotMatchedInCallOrder {
                    name: F::NAME,
                    inputs_debug: F::debug_inputs(inputs),
                    actual_call_order: error::CallOrder(current_call_index),
                    pat_index,
                });
            }

            Ok(Some((pat_index, pattern_by_call_index)))
        }
        PatternMatchMode::InAnyOrder => Ok(mock_impl
            .patterns()
            .iter()
            .enumerate()
            .find(|(_, pattern)| (*pattern.input_matcher)(inputs))),
    }
}

fn select_responder_for_call<F: MockFn>(pat: &CallPattern<F>) -> Option<&Responder<F>> {
    let call_index = pat.non_generic.increase_call_counter();

    let mut responder = None;

    for call_index_responder in pat.responders.iter() {
        if call_index_responder.response_index > call_index {
            break;
        }

        responder = Some(&call_index_responder.responder)
    }

    responder
}
