use crate::call_pattern::*;
use crate::clause::{self, ClauseSealed, TerminalClause};
use crate::fn_mocker::PatternMatchMode;
use crate::property::*;
use crate::*;

use std::marker::PhantomData;
use std::panic;

pub(crate) struct DynCallPatternBuilder {
    pub pattern_match_mode: PatternMatchMode,
    pub input_matcher: DynInputMatcher,
    pub responders: Vec<DynCallOrderResponder>,
    pub responders2: Vec<DynCallOrderResponder2>,
    pub count_expectation: counter::CallCountExpectation,
    pub current_response_index: usize,
}

impl DynCallPatternBuilder {
    pub fn new(pattern_match_mode: PatternMatchMode, input_matcher: DynInputMatcher) -> Self {
        Self {
            pattern_match_mode,
            input_matcher,
            responders: vec![],
            responders2: vec![],
            count_expectation: Default::default(),
            current_response_index: 0,
        }
    }
}

pub(crate) enum BuilderWrapper<'p> {
    Borrowed(&'p mut DynCallPatternBuilder),
    Owned(DynCallPatternBuilder),
    Stolen,
}

impl<'p> BuilderWrapper<'p> {
    fn steal(&mut self) -> BuilderWrapper<'p> {
        let mut stolen = BuilderWrapper::Stolen;
        std::mem::swap(self, &mut stolen);
        stolen
    }

    pub fn inner(&self) -> &DynCallPatternBuilder {
        match self {
            Self::Borrowed(builder) => builder,
            Self::Owned(builder) => builder,
            Self::Stolen => panic!("builder stolen"),
        }
    }

    fn inner_mut(&mut self) -> &mut DynCallPatternBuilder {
        match self {
            Self::Borrowed(builder) => builder,
            Self::Owned(builder) => builder,
            Self::Stolen => panic!("builder stolen"),
        }
    }

    fn push_responder(&mut self, responder: DynResponder) {
        let dyn_builder = self.inner_mut();
        dyn_builder.responders.push(DynCallOrderResponder {
            response_index: dyn_builder.current_response_index,
            responder,
        });
    }

    fn push_responder2(&mut self, responder: DynResponder2) {
        let dyn_builder = self.inner_mut();
        dyn_builder.responders2.push(DynCallOrderResponder2 {
            response_index: dyn_builder.current_response_index,
            responder,
        })
    }

    /// Note: must be called after `push_responder`
    fn quantify(&mut self, times: usize, exactness: counter::Exactness) {
        let mut builder = self.inner_mut();

        builder.count_expectation.add_to_minimum(times, exactness);
        builder.current_response_index += times;
    }

    fn into_owned(self) -> DynCallPatternBuilder {
        match self {
            Self::Owned(owned) => owned,
            _ => panic!("Tried to turn a non-owned pattern builder into owned"),
        }
    }
}

/// Builder for defining a series of cascading call patterns on a specific [MockFn].
pub struct Each<F: MockFn> {
    patterns: Vec<DynCallPatternBuilder>,
    mock_fn: PhantomData<F>,
}

impl<F> Each<F>
where
    F: MockFn + 'static,
{
    /// Define the next call pattern, given some input matcher.
    ///
    /// The new call pattern will be matched after any previously defined call patterns on the same [Each] instance.
    ///
    /// The method returns a [DefineMultipleResponses], which is used to define how unimock responds to the matched call.
    pub fn call<'e>(
        &'e mut self,
        matching_fn: &dyn Fn(&mut Matching<F>),
    ) -> DefineMultipleResponses<'e, F, InAnyOrder> {
        self.patterns.push(DynCallPatternBuilder::new(
            PatternMatchMode::InAnyOrder,
            DynInputMatcher::from_matching_fn(matching_fn),
        ));

        DefineMultipleResponses {
            builder: BuilderWrapper::Borrowed(self.patterns.last_mut().unwrap()),
            mock_fn: PhantomData,
            ordering: InAnyOrder,
        }
    }

    pub(crate) fn new() -> Self {
        Self {
            patterns: vec![],
            mock_fn: PhantomData,
        }
    }
}

impl<F> ClauseSealed for Each<F>
where
    F: MockFn + 'static,
{
    fn deconstruct(self, sink: &mut dyn clause::TerminalSink) -> Result<(), String> {
        if self.patterns.is_empty() {
            return Err("Stub contained no call patterns".to_string());
        }

        for builder in self.patterns.into_iter() {
            sink.put_terminal(TerminalClause {
                dyn_mock_fn: DynMockFn::new::<F>(),
                builder,
            })?;
        }

        Ok(())
    }
}

/// A matched call pattern, ready for defining a single response.
pub struct DefineResponse<'p, F: MockFn, O: Ordering> {
    builder: BuilderWrapper<'p>,
    mock_fn: PhantomData<F>,
    ordering: O,
}

impl<'p, F, O> DefineResponse<'p, F, O>
where
    F: MockFn + 'static,
    O: Ordering,
{
    /// Specify the output of the call pattern by providing a value.
    /// The output type cannot contain non-static references.
    /// It must also be [Send] and [Sync] because unimock needs to store it.
    ///
    /// Unless explicitly configured on the returned [QuantifyReturnValue], the return value specified here
    ///     can be returned only once, because this method does not require a [Clone] bound.
    /// To be able to return this value multiple times, quantify it explicitly.
    pub fn returns<V: Into<F::Output>>(self, value: V) -> QuantifyReturnValue<'p, F, O>
    where
        F::Output: Send + Sync + Sized + 'static,
    {
        QuantifyReturnValue {
            builder: self.builder,
            value: Some(value.into()),
            mock_fn: self.mock_fn,
            ordering: self.ordering,
        }
    }
}

/// A matched call pattern, ready for defining multiple response, requiring return values to implement [Clone].
pub struct DefineMultipleResponses<'p, F: MockFn, O: Ordering> {
    builder: BuilderWrapper<'p>,
    mock_fn: PhantomData<F>,
    ordering: O,
}

impl<'p, F, O> DefineMultipleResponses<'p, F, O>
where
    F: MockFn + 'static,
    O: Ordering,
{
    /// Specify the output of the call pattern by providing a value.
    /// The output type cannot contain non-static references.
    /// It must also be [Send] and [Sync] because unimock needs to store it, and [Clone] because it should be able to be returned multiple times.
    pub fn returns<V: Into<F::Output>>(mut self, value: V) -> Quantify<'p, F, O>
    where
        F::Output: Clone + Send + Sync + Sized + 'static,
    {
        let value = value.into();
        self.builder.push_responder(
            ValueResponder::<F> {
                stored_value: Box::new(StoredValueSlot(value)),
            }
            .into_dyn_responder(),
        );
        self.quantify()
    }
}

macro_rules! define_response_common_impl {
    ($typename:ident) => {
        impl<'p, F, O> $typename<'p, F, O>
        where
            F: MockFn + 'static,
            O: Ordering,
        {
            /// Create a new owned call pattern match.
            pub(crate) fn with_owned_builder(
                input_matcher: DynInputMatcher,
                pattern_match_mode: PatternMatchMode,
                ordering: O,
            ) -> Self {
                Self {
                    builder: BuilderWrapper::Owned(DynCallPatternBuilder::new(
                        pattern_match_mode,
                        input_matcher,
                    )),
                    mock_fn: PhantomData,
                    ordering,
                }
            }

            /// Specify the output of the call pattern by calling `Default::default()`.
            pub fn returns_default(mut self) -> Quantify<'p, F, O>
            where
                F::Output: Default,
            {
                self.builder.push_responder(
                    ClosureResponder::<F> {
                        func: Box::new(|_| Default::default()),
                    }
                    .into_dyn_responder(),
                );
                self.quantify()
            }

            /// Specify the output of the call to be a borrow of the provided value.
            /// This works well when the lifetime of the returned reference is the same as `self`.
            /// Using this for `'static` references will produce a runtime error. For static references, use [DefineResponse::returns_static].
            pub fn returns_ref<T>(mut self, value: T) -> Quantify<'p, F, O>
            where
                T: std::borrow::Borrow<F::Output> + Sized + Send + Sync + 'static,
            {
                self.builder.push_responder(
                    BorrowableResponder::<F> {
                        borrowable: Box::new(value),
                    }
                    .into_dyn_responder(),
                );
                self.quantify()
            }

            /// Specify the output of the call to be a reference to static value.
            /// This must be used when the returned reference in the mocked trait is `'static`.
            pub fn returns_static(mut self, value: &'static F::Output) -> Quantify<'p, F, O>
            where
                F::Output: Send + Sync + 'static,
            {
                self.builder.push_responder(
                    StaticRefClosureResponder::<F> {
                        func: Box::new(move |_| value),
                    }
                    .into_dyn_responder(),
                );
                self.quantify()
            }

            /// Specify the output of the call pattern by invoking the given closure that can then compute it based on input parameters.
            pub fn answers<A, R>(mut self, func: A) -> Quantify<'p, F, O>
            where
                A: (for<'i> Fn(F::Inputs<'i>) -> R) + Send + Sync + 'static,
                R: Into<F::Output>,
                F::Output: Sized,
            {
                self.builder.push_responder(
                    ClosureResponder::<F> {
                        func: Box::new(move |inputs| func(inputs).into()),
                    }
                    .into_dyn_responder(),
                );
                self.quantify()
            }

            /// Specify the output of the call pattern to be a static reference to leaked memory.
            ///
            /// The value may be based on the value of input parameters.
            ///
            /// This version will produce a new memory leak for _every invocation_ of the answer function.
            ///
            /// This method should only be used when computing a reference based
            /// on input parameters is necessary, which should not be a common use case.
            pub fn answers_leaked_ref<A, R>(mut self, func: A) -> Quantify<'p, F, O>
            where
                A: (for<'i> Fn(F::Inputs<'i>) -> R) + Send + Sync + 'static,
                R: std::borrow::Borrow<F::Output> + 'static,
                F::Output: Sized,
            {
                self.builder.push_responder(
                    StaticRefClosureResponder::<F> {
                        func: Box::new(move |inputs| {
                            let value = func(inputs);
                            let leaked_ref = Box::leak(Box::new(value));
                            <R as std::borrow::Borrow<F::Output>>::borrow(leaked_ref)
                        }),
                    }
                    .into_dyn_responder(),
                );
                self.quantify()
            }

            /// Prevent this call pattern from succeeding by explicitly panicking with a custom message.
            pub fn panics(mut self, message: impl Into<String>) -> Quantify<'p, F, O> {
                self.builder
                    .push_responder(DynResponder::Panic(message.into()));
                self.quantify()
            }

            /// Instruct this call pattern to invoke its corresponding `unmocked` function.
            ///
            /// For this to work, the mocked trait must be configured with an `unmock_with=[..]` parameter.
            /// If unimock doesn't find a way to unmock the function, this will panic when the function is called.
            pub fn unmocked(mut self) -> Quantify<'p, F, O> {
                self.builder.push_responder(DynResponder::Unmock);
                self.quantify()
            }

            fn quantify(self) -> Quantify<'p, F, O> {
                Quantify {
                    builder: self.builder,
                    mock_fn: PhantomData,
                    ordering: self.ordering,
                }
            }
        }
    };
}

define_response_common_impl!(DefineResponse);
define_response_common_impl!(DefineMultipleResponses);

/// Builder for defining how a call pattern with an explicit return value gets verified with regards to quantification/counting.
pub struct QuantifyReturnValue<'p, F, O>
where
    F: MockFn,
    F::Output: Sized + Send + Sync,
{
    builder: BuilderWrapper<'p>,
    value: Option<F::Output>,
    mock_fn: PhantomData<F>,
    ordering: O,
}

impl<'p, F, O> QuantifyReturnValue<'p, F, O>
where
    F: MockFn,
    F::Output: Sized + Send + Sync,
    O: Copy,
{
    /// Expect this call pattern to be called exactly once.
    ///
    /// This is the only quantifier that works together with return values that don't implement [Clone].
    pub fn once(mut self) -> QuantifiedResponse<'p, F, O, Exact> {
        self.builder.push_responder(
            ValueResponder::<F> {
                stored_value: Box::new(StoredValueSlotOnce::new(self.value.take().unwrap())),
            }
            .into_dyn_responder(),
        );
        self.builder.quantify(1, counter::Exactness::Exact);
        QuantifiedResponse {
            builder: self.builder.steal(),
            mock_fn: PhantomData,
            ordering: self.ordering,
            _repetition: Exact,
        }
    }

    /// Expect this call pattern to be called exactly the specified number of times.
    pub fn n_times(mut self, times: usize) -> QuantifiedResponse<'p, F, O, Exact>
    where
        F::Output: Clone,
    {
        self.builder.push_responder(
            ValueResponder::<F> {
                stored_value: Box::new(StoredValueSlot(self.value.take().unwrap())),
            }
            .into_dyn_responder(),
        );
        self.builder.quantify(times, counter::Exactness::Exact);
        QuantifiedResponse {
            builder: self.builder.steal(),
            mock_fn: PhantomData,
            ordering: self.ordering,
            _repetition: Exact,
        }
    }

    /// Expect this call pattern to be called at least the specified number of times.
    pub fn at_least_times(mut self, times: usize) -> QuantifiedResponse<'p, F, O, AtLeast>
    where
        F::Output: Clone,
    {
        self.builder.push_responder(
            ValueResponder::<F> {
                stored_value: Box::new(StoredValueSlot(self.value.take().unwrap())),
            }
            .into_dyn_responder(),
        );
        self.builder.quantify(times, counter::Exactness::AtLeast);
        QuantifiedResponse {
            builder: self.builder.steal(),
            mock_fn: PhantomData,
            ordering: self.ordering,
            _repetition: AtLeast,
        }
    }
}

impl<'p, F, O> ClauseSealed for QuantifyReturnValue<'p, F, O>
where
    F: MockFn,
    F::Output: Sized + Send + Sync,
    O: Copy + Ordering,
{
    fn deconstruct(self, sink: &mut dyn clause::TerminalSink) -> Result<(), String> {
        self.once().deconstruct(sink)
    }
}

/// The drop implementation of this is intended to run when used in a stubbing clause,
/// when the call pattern is left unquantified by the user.
///
/// In that case, it is only able to return once, because no [Clone] bound has been
/// part of any construction step.
impl<'p, F, O> Drop for QuantifyReturnValue<'p, F, O>
where
    F: MockFn,
    F::Output: Sized + Send + Sync,
{
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            self.builder.push_responder(
                ValueResponder::<F> {
                    stored_value: Box::new(StoredValueSlotOnce::new(value)),
                }
                .into_dyn_responder(),
            );
        }
    }
}

/// Builder for defining how a call pattern gets verified with regards to quantification/counting.
pub struct Quantify<'p, F: MockFn, O> {
    builder: BuilderWrapper<'p>,
    mock_fn: PhantomData<F>,
    ordering: O,
}

impl<'p, F, O> Quantify<'p, F, O>
where
    F: MockFn + 'static,
    O: Ordering,
{
    /// Expect this call pattern to be called exactly once.
    pub fn once(mut self) -> QuantifiedResponse<'p, F, O, Exact> {
        self.builder.quantify(1, counter::Exactness::Exact);
        self.into_exact()
    }

    /// Expect this call pattern to be called exactly the specified number of times.
    pub fn n_times(mut self, times: usize) -> QuantifiedResponse<'p, F, O, Exact> {
        self.builder.quantify(times, counter::Exactness::Exact);
        self.into_exact()
    }

    /// Expect this call pattern to be called at least the specified number of times.
    pub fn at_least_times(mut self, times: usize) -> QuantifiedResponse<'p, F, O, AtLeast> {
        self.builder.quantify(times, counter::Exactness::AtLeast);
        QuantifiedResponse {
            builder: self.builder,
            mock_fn: PhantomData,
            ordering: self.ordering,
            _repetition: AtLeast,
        }
    }

    fn into_exact(self) -> QuantifiedResponse<'p, F, O, Exact> {
        QuantifiedResponse {
            builder: self.builder,
            mock_fn: PhantomData,
            ordering: self.ordering,
            _repetition: Exact,
        }
    }
}

impl<'p, F, O> ClauseSealed for Quantify<'p, F, O>
where
    F: MockFn + 'static,
    O: Ordering,
{
    fn deconstruct(mut self, sink: &mut dyn clause::TerminalSink) -> Result<(), String> {
        if self.builder.inner().pattern_match_mode == PatternMatchMode::InOrder {
            self.builder.quantify(1, counter::Exactness::Exact);
        }

        sink.put_terminal(TerminalClause {
            dyn_mock_fn: DynMockFn::new::<F>(),
            builder: self.builder.into_owned(),
        })
    }
}

/// An exactly quantified response, i.e. the number of times it is expected to respond is an exact number.
pub struct QuantifiedResponse<'p, F: MockFn, O, R> {
    builder: BuilderWrapper<'p>,
    mock_fn: PhantomData<F>,
    ordering: O,
    _repetition: R,
}

impl<'p, F, O, R> QuantifiedResponse<'p, F, O, R>
where
    F: MockFn + 'static,
    O: Ordering,
    R: Repetition,
{
    /// Prepare to set up a new response, which will take effect after the current response has been yielded.
    /// In order to make an output sequence, the preceding output must be exactly quantified.
    pub fn then(mut self) -> DefineMultipleResponses<'p, F, O>
    where
        R: Repetition<Kind = Exact>,
    {
        // Opening for a new response, which will be non-exactly quantified unless otherwise specified, set the exactness to AtLeastPlusOne now.
        // The reason it is AtLeastPlusOne is the additive nature.
        // We do not want to add anything to the number now, because it could be added to later in Quantify.
        // We just want to express that when using `then`, it should be called at least one time, if not `then` would be unnecessary.
        self.builder
            .inner_mut()
            .count_expectation
            .add_to_minimum(0, counter::Exactness::AtLeastPlusOne);

        DefineMultipleResponses {
            builder: self.builder,
            mock_fn: PhantomData,
            ordering: self.ordering,
        }
    }
}

impl<'p, F, O, R> ClauseSealed for QuantifiedResponse<'p, F, O, R>
where
    F: MockFn + 'static,
    O: Ordering,
    R: Repetition,
{
    fn deconstruct(self, sink: &mut dyn clause::TerminalSink) -> Result<(), String> {
        sink.put_terminal(TerminalClause {
            dyn_mock_fn: DynMockFn::new::<F>(),
            builder: self.builder.into_owned(),
        })
    }
}

pub mod v2 {
    use super::*;
    use crate::output::*;

    pub struct DefineResponse<'p, F: MockFn2, O: Ordering> {
        builder: BuilderWrapper<'p>,
        mock_fn: PhantomData<F>,
        ordering: O,
    }

    pub struct QuantifyTodo<'p, F: MockFn2, O: Ordering> {
        pub(crate) builder: BuilderWrapper<'p>,
        mock_fn: PhantomData<F>,
        ordering: O,
    }

    impl<'p, F: MockFn2, O: Ordering> DefineResponse<'p, F, O> {
        pub(crate) fn with_owned_builder(
            input_matcher: DynInputMatcher,
            pattern_match_mode: PatternMatchMode,
            ordering: O,
        ) -> Self {
            Self {
                builder: BuilderWrapper::Owned(DynCallPatternBuilder::new(
                    pattern_match_mode,
                    input_matcher,
                )),
                mock_fn: PhantomData,
                ordering,
            }
        }

        fn quantify(self) -> QuantifyTodo<'p, F, O> {
            QuantifyTodo {
                builder: self.builder,
                mock_fn: PhantomData,
                ordering: self.ordering,
            }
        }
    }

    impl<'p, F: MockFn2, O: Ordering> DefineResponse<'p, F, O>
    where
        F::Output: OwnedOutput,
        <F::Output as Output>::Type: Clone + Send + Sync,
    {
        // FIXME: Should return a Quantify object and not require clone
        pub fn returns(
            mut self,
            value: impl Into<<F::Output as Output>::Type>,
        ) -> QuantifyTodo<'p, F, O> {
            let value = value.into();
            self.builder.push_responder2(
                OwnedResponder2::<F> {
                    stored_value: Box::new(StoredValueSlot(value)),
                }
                .into_dyn_responder(),
            );
            self.quantify()
        }
    }

    impl<'p, F: MockFn2, O: Ordering> DefineResponse<'p, F, O>
    where
        F::Output: RefOutput,
        <F::Output as Output>::Type: Send + Sync,
    {
        pub fn returns_ref<T: ?Sized + Send + Sync>(
            mut self,
            value: impl std::borrow::Borrow<T> + Send + Sync,
        ) -> QuantifyTodo<'p, F, O>
        where
            F::Output: IntoRefOutput<T>,
        {
            let borrowable = <F::Output as IntoRefOutput<T>>::into_ref_output(value);
            self.builder
                .push_responder2(RefResponder2::<F> { borrowable }.into_dyn_responder());
            self.quantify()
        }
    }

    impl<'p, F: MockFn2, O: Ordering> DefineResponse<'p, F, O>
    where
        for<'u> F::OutputOld<'u>: StaticRefOutputOld,
        <F::OutputOld<'static> as OutputOld<'static>>::Type: Send + Sync + Copy + 'static,
    {
        pub fn returns(
            mut self,
            value: <F::OutputOld<'static> as OutputOld<'static>>::Type,
        ) -> QuantifyTodo<'p, F, O> {
            self.builder.push_responder2(
                StaticRefClosureResponder2::<F> {
                    func: Box::new(move |_| value),
                }
                .into_dyn_responder(),
            );
            self.quantify()
        }
    }

    impl<'p, F: MockFn2, O: Ordering> DefineResponse<'p, F, O>
    where
        for<'u> F::OutputOld<'u>: ComplexOutputOld<'u>,
        for<'u> <F::OutputOld<'u> as StoreOutputOld<'u>>::Stored: Clone + Send + Sync,
    {
        pub fn returns(
            mut self,
            value: impl Into<<F::OutputOld<'static> as StoreOutputOld<'static>>::Stored>,
        ) -> QuantifyTodo<'p, F, O> {
            let value = value.into();
            self.builder.push_responder2(
                ComplexValueResponder2 {
                    stored_value: Box::new(StoredValueSlot(value)),
                }
                .into_dyn_responder(),
            );
            self.quantify()
        }
    }
}
