use crate::private::lib::{Box, String, Vec};
use core::any::Any;

use crate::build;
use crate::cell::{Cell, CloneCell, FactoryCell};
use crate::debug;
use crate::output::{Respond, ResponderError};
use crate::private::MismatchReporter;
use crate::*;

#[derive(Clone, Copy)]
pub(crate) struct PatIndex(pub usize);

#[derive(Clone, Copy)]
pub(crate) struct InputIndex(pub usize);

impl core::fmt::Display for PatIndex {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

pub enum PatternError {
    Downcast,
    NoMatcherFunction,
}

pub type PatternResult<T> = Result<T, PatternError>;

pub(crate) type AnyBox = Box<dyn Any + Send + Sync + 'static>;

fn downcast_box<T: 'static>(any_box: &AnyBox) -> PatternResult<&T> {
    any_box.downcast_ref().ok_or(PatternError::Downcast)
}

pub(crate) struct CallPattern {
    pub input_matcher: DynInputMatcher,
    pub responders: Vec<DynCallOrderResponder>,
    pub ordered_call_index_range: core::ops::Range<usize>,
    pub call_counter: counter::CallCounter,
}

impl CallPattern {
    pub fn match_inputs<F: MockFn>(
        &self,
        inputs: &F::Inputs<'_>,
        mismatch_reporter: Option<&mut MismatchReporter>,
    ) -> PatternResult<bool> {
        match (&self.input_matcher.dyn_matching_fn, mismatch_reporter) {
            (Some(DynMatchingFn(f)), Some(reporter)) => {
                Ok((downcast_box::<MatchingFn<F>>(f)?.0)(inputs, reporter))
            }
            (Some(DynMatchingFn(f)), None) => Ok((downcast_box::<MatchingFn<F>>(f)?.0)(
                inputs,
                &mut MismatchReporter::new_disabled(),
            )),
            (None, _) => Err(PatternError::NoMatcherFunction),
        }
    }

    pub fn debug_location(&self, pat_index: PatIndex) -> debug::CallPatternLocation {
        if let Some(debug) = self.input_matcher.matcher_debug {
            debug::CallPatternLocation::Debug(debug)
        } else {
            debug::CallPatternLocation::PatIndex(pat_index)
        }
    }

    pub fn next_responder(&self) -> Option<&DynResponder> {
        find_responder_by_call_index(&self.responders, self.call_counter.fetch_add())
    }
}

pub(crate) struct DynInputMatcher {
    dyn_matching_fn: Option<DynMatchingFn>,
    pub(crate) matcher_debug: Option<debug::InputMatcherDebug>,
}

impl DynInputMatcher {
    pub fn from_matching_fn<F: MockFn>(matching_fn: &dyn Fn(&mut Matching<F>)) -> Self {
        let mut builder = Matching::new();
        matching_fn(&mut builder);

        Self {
            dyn_matching_fn: builder.matching_fn.map(|f| DynMatchingFn(Box::new(f))),
            matcher_debug: builder.matcher_debug,
        }
    }
}

struct DynMatchingFn(AnyBox);

pub(crate) struct MatchingFn<F: MockFn>(
    #[allow(clippy::type_complexity)]
    pub  Box<dyn (for<'i> Fn(&F::Inputs<'i>, &mut MismatchReporter) -> bool) + Send + Sync>,
);

pub(crate) struct DynCallOrderResponder {
    pub response_index: usize,
    pub responder: DynResponder,
}

pub(crate) enum DynResponder {
    Cell(DynCellResponder),
    Borrow(DynBorrowResponder),
    Function(DynFunctionResponder),
    Panic(String),
    Unmock,
    CallDefaultImpl,
}

impl DynResponder {
    #[cfg(any(feature = "std", feature = "spin-lock"))]
    pub fn new_cell<F: MockFn>(
        response: <F::Response as Respond>::Type,
    ) -> Result<Self, ResponderError>
    where
        <F::Response as Respond>::Type: Send + Sync + 'static,
    {
        let response = crate::private::MutexIsh::new(Some(response));
        Ok(CellResponder::<F> {
            cell: Box::new(FactoryCell::new(move || {
                response.locked(|option| option.take())
            })),
        }
        .into_dyn_responder())
    }

    #[cfg(not(any(feature = "std", feature = "spin-lock")))]
    pub fn new_cell<F: MockFn>(_: <F::Response as Respond>::Type) -> Result<Self, ResponderError>
    where
        <F::Response as Respond>::Type: Send + Sync + 'static,
    {
        Err(ResponderError::NoMutexApi)
    }

    pub fn new_clone_cell<F: MockFn>(response: <F::Response as Respond>::Type) -> Self
    where
        <F::Response as Respond>::Type: Clone + Send + Sync + 'static,
    {
        CellResponder::<F> {
            cell: Box::new(CloneCell(response)),
        }
        .into_dyn_responder()
    }

    pub fn new_clone_factory_cell<F: MockFn>(
        clone_fn: impl Fn() -> Option<<F::Response as Respond>::Type> + Send + Sync + 'static,
    ) -> Self
    where
        <F::Response as Respond>::Type: Send + Sync + 'static,
    {
        CellResponder::<F> {
            cell: Box::new(FactoryCell::new(clone_fn)),
        }
        .into_dyn_responder()
    }

    pub fn new_borrow<F: MockFn>(response: <F::Response as Respond>::Type) -> Self
    where
        <F::Response as Respond>::Type: Send + Sync,
    {
        BorrowResponder::<F> {
            borrowable: response,
        }
        .into_dyn_responder()
    }
}

pub(crate) struct DynCellResponder(AnyBox);
pub(crate) struct DynBorrowResponder(AnyBox);
pub(crate) struct DynFunctionResponder(AnyBox);

pub trait DowncastResponder<F: MockFn> {
    type Downcasted;

    fn downcast(&self) -> PatternResult<&Self::Downcasted>;
}

impl<F: MockFn> DowncastResponder<F> for DynCellResponder {
    type Downcasted = CellResponder<F>;

    fn downcast(&self) -> PatternResult<&Self::Downcasted> {
        downcast_box(&self.0)
    }
}

impl<F: MockFn> DowncastResponder<F> for DynBorrowResponder {
    type Downcasted = BorrowResponder<F>;

    fn downcast(&self) -> PatternResult<&Self::Downcasted> {
        downcast_box(&self.0)
    }
}

impl<F: MockFn> DowncastResponder<F> for DynFunctionResponder {
    type Downcasted = FunctionResponder<F>;

    fn downcast(&self) -> PatternResult<&Self::Downcasted> {
        downcast_box(&self.0)
    }
}

pub(crate) struct CellResponder<F: MockFn> {
    pub cell: Box<dyn Cell<<F::Response as Respond>::Type>>,
}

pub(crate) struct BorrowResponder<F: MockFn> {
    pub borrowable: <F::Response as Respond>::Type,
}

pub(crate) struct FunctionResponder<F: MockFn> {
    #[allow(clippy::type_complexity)]
    pub func: Box<
        dyn (Fn(F::Inputs<'_>, build::AnswerContext<'_, '_, '_, F>) -> <F::Response as Respond>::Type)
            + Send
            + Sync,
    >,
}

impl<F: MockFn> CellResponder<F> {
    pub fn into_dyn_responder(self) -> DynResponder {
        DynResponder::Cell(DynCellResponder(Box::new(self)))
    }
}

impl<F: MockFn> BorrowResponder<F>
where
    <F::Response as Respond>::Type: Send + Sync,
{
    pub fn into_dyn_responder(self) -> DynResponder {
        DynResponder::Borrow(DynBorrowResponder(Box::new(self)))
    }
}

impl<F: MockFn> FunctionResponder<F> {
    pub fn into_dyn_responder(self) -> DynResponder {
        DynResponder::Function(DynFunctionResponder(Box::new(self)))
    }
}

fn find_responder_by_call_index(
    responders: &[DynCallOrderResponder],
    call_index: usize,
) -> Option<&DynResponder> {
    if responders.is_empty() {
        return None;
    }

    let index_result =
        responders.binary_search_by(|responder| responder.response_index.cmp(&call_index));

    Some(match index_result {
        Ok(index) => &responders[index].responder,
        Err(insert_index) => &responders[insert_index - 1].responder,
    })
}

#[cfg(test)]
mod tests {
    use crate::private::lib::{vec, ToString};

    use super::*;

    #[test]
    fn should_select_responder_with_lower_call_index() {
        let responders = vec![
            DynCallOrderResponder {
                response_index: 0,
                responder: DynResponder::Panic("0".to_string()),
            },
            DynCallOrderResponder {
                response_index: 5,
                responder: DynResponder::Panic("5".to_string()),
            },
        ];

        fn find_msg(responders: &[DynCallOrderResponder], call_index: usize) -> Option<&str> {
            find_responder_by_call_index(responders, call_index).map(|responder| match responder {
                DynResponder::Panic(msg) => msg.as_str(),
                _ => panic!(),
            })
        }

        assert_eq!(find_msg(&[], 42), None);
        assert_eq!(find_msg(&responders, 0), Some("0"));
        assert_eq!(find_msg(&responders, 4), Some("0"));
        assert_eq!(find_msg(&responders, 5), Some("5"));
        assert_eq!(find_msg(&responders, 7), Some("5"));
    }
}
