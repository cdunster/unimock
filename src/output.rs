use core::borrow::Borrow;

use crate::private::lib::Box;
use crate::{call_pattern::DynResponder, value_chain::ValueChain, MockFn, Responder};

#[derive(Debug)]
#[doc(hidden)]
pub enum ResponderError {
    OwnershipRequired,
    NoMutexApi,
}

type OutputResult<T> = Result<T, ResponderError>;

/// Trait for responding to function calls.
pub trait Respond {
    /// The type of the response, as stored temporarily inside Unimock.
    type Type: 'static;
}

/// Trait for values that can be converted into responses.
pub trait IntoResponse<R: Respond> {
    // Convert this type into the output type.
    #[doc(hidden)]
    fn into_response(self) -> <R as Respond>::Type;
}

/// Types that may converted into a responder that responds once.
///
/// This can be implemented by types that do not implement `Clone`.
pub trait IntoOnceResponder<R: Respond>: IntoResponse<R> {
    #[doc(hidden)]
    fn into_once_responder<F: MockFn<Response = R>>(self) -> OutputResult<Responder>;
}

/// Types that may converted into a responder that responds any number of times.
pub trait IntoCloneResponder<R: Respond>: IntoOnceResponder<R> {
    #[doc(hidden)]
    fn into_clone_responder<F: MockFn<Response = R>>(self) -> OutputResult<Responder>;
}

/// Trait that describes the output of a mocked function, and how responses are converted into that type.
///
/// The trait uses the 'u lifetime, which is the lifetime of unimock itself.
/// This way it's possible to borrow values stored inside the instance.
pub trait Output<'u, R: Respond> {
    /// The type of the output compatible with the function signature.
    type Type;

    #[doc(hidden)]
    fn from_response(response: R::Type, value_chain: &'u ValueChain) -> Self::Type;

    #[doc(hidden)]
    fn try_from_borrowed_response(response: &'u R::Type) -> OutputResult<Self::Type>;
}

#[doc(hidden)]
pub struct Owned<T>(core::marker::PhantomData<T>);

// This type describes a function response that is a reference borrowed from `Self`.
#[doc(hidden)]
pub struct Borrowed<T: ?Sized + 'static>(core::marker::PhantomData<T>);

#[doc(hidden)]
pub struct StaticRef<T: ?Sized>(core::marker::PhantomData<T>);

// This type describes a function response that is a mix of owned and borrowed data.
//
// The typical example is `Option<&T>`.
#[doc(hidden)]
pub struct Mixed<T>(core::marker::PhantomData<T>);

type BoxBorrow<T> = Box<dyn Borrow<T> + Send + Sync>;

mod owned {
    use super::*;

    impl<T: 'static> Respond for Owned<T> {
        type Type = T;
    }

    impl<T0, T: 'static> IntoResponse<Owned<T>> for T0
    where
        T0: Into<T>,
    {
        fn into_response(self) -> <Owned<T> as Respond>::Type {
            self.into()
        }
    }

    impl<T0, T: Send + Sync + 'static> IntoOnceResponder<Owned<T>> for T0
    where
        T0: Into<T>,
    {
        fn into_once_responder<F: MockFn<Response = Owned<T>>>(self) -> OutputResult<Responder> {
            let response = <T0 as IntoResponse<Owned<T>>>::into_response(self);
            Ok(Responder(DynResponder::new_cell::<F>(response)?))
        }
    }

    impl<T0, T: Clone + Send + Sync + 'static> IntoCloneResponder<Owned<T>> for T0
    where
        T0: Into<T>,
    {
        fn into_clone_responder<F: MockFn<Response = Owned<T>>>(self) -> OutputResult<Responder> {
            let response = <T0 as IntoResponse<Owned<T>>>::into_response(self);
            Ok(Responder(DynResponder::new_clone_cell::<F>(response)))
        }
    }

    impl<'u, T: 'static> Output<'u, Self> for Owned<T> {
        type Type = T;

        fn from_response(response: <Self as Respond>::Type, _: &'u ValueChain) -> Self::Type {
            response
        }

        fn try_from_borrowed_response(_: &'u <Self as Respond>::Type) -> OutputResult<Self::Type> {
            Err(ResponderError::OwnershipRequired)
        }
    }
}

mod borrowed {
    use super::*;

    impl<T: ?Sized + 'static> Respond for Borrowed<T> {
        type Type = Box<dyn Borrow<T> + Send + Sync>;
    }

    impl<T0, T> IntoResponse<Borrowed<T>> for T0
    where
        T0: Borrow<T> + Send + Sync + 'static,
        T: ?Sized + 'static,
    {
        fn into_response(self) -> <Borrowed<T> as Respond>::Type {
            Box::new(self)
        }
    }

    impl<T0, T> IntoOnceResponder<Borrowed<T>> for T0
    where
        T0: Borrow<T> + Send + Sync + 'static,
        T: ?Sized + 'static,
    {
        fn into_once_responder<F: MockFn<Response = Borrowed<T>>>(self) -> OutputResult<Responder> {
            let response = <T0 as IntoResponse<Borrowed<T>>>::into_response(self);
            Ok(Responder(DynResponder::new_borrow::<F>(response)))
        }
    }

    impl<T0, T> IntoCloneResponder<Borrowed<T>> for T0
    where
        T0: Borrow<T> + Send + Sync + 'static,
        T: ?Sized + 'static,
    {
        fn into_clone_responder<F: MockFn<Response = Borrowed<T>>>(
            self,
        ) -> OutputResult<Responder> {
            <T0 as IntoOnceResponder<Borrowed<T>>>::into_once_responder::<F>(self)
        }
    }

    impl<'u, T: ?Sized + 'static> Output<'u, Self> for Borrowed<T> {
        type Type = &'u T;

        fn from_response(
            response: <Self as Respond>::Type,
            value_chain: &'u ValueChain,
        ) -> Self::Type {
            let value_ref = value_chain.add(response);

            value_ref.as_ref().borrow()
        }

        fn try_from_borrowed_response(
            response: &'u <Self as Respond>::Type,
        ) -> OutputResult<Self::Type> {
            Ok(response.as_ref().borrow())
        }
    }
}

mod static_ref {
    use super::*;

    impl<T: ?Sized + 'static> Respond for StaticRef<T> {
        type Type = &'static T;
    }

    impl<T: ?Sized + Send + Sync + 'static> IntoResponse<StaticRef<T>> for &'static T {
        fn into_response(self) -> <StaticRef<T> as Respond>::Type {
            self
        }
    }

    impl<T: ?Sized + Send + Sync + 'static> IntoOnceResponder<StaticRef<T>> for &'static T {
        fn into_once_responder<F: MockFn<Response = StaticRef<T>>>(
            self,
        ) -> OutputResult<Responder> {
            let response = <Self as IntoResponse<StaticRef<T>>>::into_response(self);
            Ok(Responder(DynResponder::new_borrow::<F>(response)))
        }
    }

    impl<T: ?Sized + Send + Sync + 'static> IntoCloneResponder<StaticRef<T>> for &'static T {
        fn into_clone_responder<F: MockFn<Response = StaticRef<T>>>(
            self,
        ) -> OutputResult<Responder> {
            <Self as IntoOnceResponder<StaticRef<T>>>::into_once_responder::<F>(self)
        }
    }

    impl<'u, T: ?Sized + 'static> Output<'u, Self> for StaticRef<T> {
        type Type = &'static T;

        fn from_response(value: <Self as Respond>::Type, _: &ValueChain) -> Self::Type {
            value
        }

        fn try_from_borrowed_response(
            value: &'u <Self as Respond>::Type,
        ) -> OutputResult<Self::Type> {
            Ok(*value)
        }
    }
}

// TODO: Generalize in mixed enum macro.
mod mixed_option {
    use super::*;

    type Mix<T> = Mixed<Option<&'static T>>;

    impl<T: ?Sized + 'static> Respond for Mix<T> {
        type Type = Option<BoxBorrow<T>>;
    }

    impl<T0, T> IntoResponse<Mix<T>> for Option<T0>
    where
        T0: Borrow<T> + Send + Sync + 'static,
        T: ?Sized + 'static,
    {
        fn into_response(self) -> <Mix<T> as Respond>::Type {
            match self {
                Some(value) => Some(Box::new(value)),
                None => None,
            }
        }
    }

    impl<T0, T> IntoOnceResponder<Mix<T>> for Option<T0>
    where
        T0: Borrow<T> + Send + Sync + 'static,
        T: ?Sized + 'static,
    {
        fn into_once_responder<F: MockFn<Response = Mix<T>>>(self) -> OutputResult<Responder> {
            let response = <Self as IntoResponse<Mix<T>>>::into_response(self);
            Ok(Responder(DynResponder::new_borrow::<F>(response)))
        }
    }

    impl<T0, T> IntoCloneResponder<Mix<T>> for Option<T0>
    where
        T0: Borrow<T> + Send + Sync + 'static,
        T: ?Sized + 'static,
    {
        fn into_clone_responder<F: MockFn<Response = Mix<T>>>(self) -> OutputResult<Responder> {
            let response = <Self as IntoResponse<Mix<T>>>::into_response(self);
            Ok(Responder(DynResponder::new_borrow::<F>(response)))
        }
    }

    impl<'u, T> Output<'u, Mix<T>> for Mixed<Option<&'u T>>
    where
        T: ?Sized + 'u,
    {
        type Type = Option<&'u T>;

        fn from_response(
            response: <Mix<T> as Respond>::Type,
            value_chain: &'u ValueChain,
        ) -> Self::Type {
            response.map(|value| value_chain.add(value).as_ref().borrow())
        }

        fn try_from_borrowed_response(
            response: &'u <Mix<T> as Respond>::Type,
        ) -> OutputResult<Self::Type> {
            Ok(response.as_ref().map(|value| value.as_ref().borrow()))
        }
    }
}

mod mixed_vec {
    use crate::private::lib::Vec;

    use super::*;

    type Mix<T> = Mixed<Vec<&'static T>>;

    impl<T: ?Sized + 'static> Respond for Mix<T> {
        type Type = Vec<BoxBorrow<T>>;
    }

    impl<T0, T> IntoResponse<Mix<T>> for Vec<T0>
    where
        T0: Borrow<T> + Send + Sync + 'static,
        T: ?Sized + 'static,
    {
        fn into_response(self) -> <Mix<T> as Respond>::Type {
            self.into_iter()
                .map(|item| -> BoxBorrow<T> { Box::new(item) })
                .collect()
        }
    }

    impl<T0, T> IntoOnceResponder<Mix<T>> for Vec<T0>
    where
        T0: Borrow<T> + Send + Sync + 'static,
        T: ?Sized + 'static,
    {
        fn into_once_responder<F: MockFn<Response = Mix<T>>>(self) -> OutputResult<Responder> {
            let response = <Self as IntoResponse<Mix<T>>>::into_response(self);
            Ok(Responder(DynResponder::new_borrow::<F>(response)))
        }
    }

    impl<T0, T> IntoCloneResponder<Mix<T>> for Vec<T0>
    where
        T0: Borrow<T> + Send + Sync + 'static,
        T: ?Sized + 'static,
    {
        fn into_clone_responder<F: MockFn<Response = Mix<T>>>(self) -> OutputResult<Responder> {
            let response = <Self as IntoResponse<Mix<T>>>::into_response(self);
            Ok(Responder(DynResponder::new_borrow::<F>(response)))
        }
    }

    impl<'u, T> Output<'u, Mix<T>> for Mixed<Vec<&'u T>>
    where
        T: ?Sized + 'u,
    {
        type Type = Vec<&'u T>;

        fn from_response(_: <Mix<T> as Respond>::Type, _: &'u ValueChain) -> Self::Type {
            panic!()
        }

        fn try_from_borrowed_response(
            response: &'u <Mix<T> as Respond>::Type,
        ) -> OutputResult<Self::Type> {
            Ok(response.iter().map(|b| b.as_ref().borrow()).collect())
        }
    }
}

// TODO: Generalize in mixed enum macro.
mod mixed_result_borrowed_t {
    use super::*;

    type Mix<T, E> = Mixed<Result<&'static T, E>>;

    impl<T: ?Sized + 'static, E: 'static> Respond for Mix<T, E> {
        type Type = Result<BoxBorrow<T>, E>;
    }

    impl<T0, T, E> IntoResponse<Mix<T, E>> for Result<T0, E>
    where
        T0: Borrow<T> + Send + Sync + 'static,
        T: ?Sized + 'static,
        E: Send + Sync + 'static,
    {
        fn into_response(self) -> <Mix<T, E> as Respond>::Type {
            match self {
                Ok(value) => Ok(Box::new(value)),
                Err(e) => Err(e),
            }
        }
    }

    impl<T0, T, E> IntoOnceResponder<Mix<T, E>> for Result<T0, E>
    where
        T0: Borrow<T> + Send + Sync + 'static,
        T: ?Sized + 'static,
        E: Send + Sync + 'static,
    {
        fn into_once_responder<F: MockFn<Response = Mix<T, E>>>(self) -> OutputResult<Responder> {
            match self {
                // In the Ok variant we make a multi-value responder out of it anyway:
                Ok(value) => Ok(Responder(DynResponder::new_borrow::<F>(Ok(Box::new(
                    value,
                ))))),
                // The Err variant can only be used once:
                Err(error) => Ok(Responder(DynResponder::new_cell::<F>(Err(error))?)),
            }
        }
    }

    impl<T0, T, E> IntoCloneResponder<Mix<T, E>> for Result<T0, E>
    where
        T0: Borrow<T> + Send + Sync + 'static,
        T: ?Sized + 'static,
        E: Clone + Send + Sync + 'static,
    {
        fn into_clone_responder<F: MockFn<Response = Mix<T, E>>>(self) -> OutputResult<Responder> {
            match self {
                // There is no `T0: Clone` bound, because it just uses the borrow responder mechanism:
                Ok(value) => Ok(Responder(DynResponder::new_borrow::<F>(Ok(Box::new(
                    value,
                ))))),
                // We have `E: Clone` because the E is in fact owned...
                Err(error) => Ok(Responder(DynResponder::new_clone_factory_cell::<F>(
                    move || Some(Err(error.clone())),
                ))),
            }
        }
    }

    impl<'u, T, E: 'static> Output<'u, Mix<T, E>> for Mixed<Result<&'u T, E>>
    where
        T: ?Sized + 'u,
    {
        type Type = Result<&'u T, E>;

        fn from_response(
            response: <Mix<T, E> as Respond>::Type,
            value_chain: &'u ValueChain,
        ) -> Self::Type {
            match response {
                Ok(value) => Ok(value_chain.add(value).as_ref().borrow()),
                Err(e) => Err(e),
            }
        }

        fn try_from_borrowed_response(
            response: &'u <Mix<T, E> as Respond>::Type,
        ) -> OutputResult<Self::Type> {
            match response {
                Ok(value) => Ok(Ok(value.as_ref().borrow())),
                // No chance of converting the E into owned here:
                Err(_) => Err(ResponderError::OwnershipRequired),
            }
        }
    }
}

macro_rules! mixed_tuples {
    ($(($t:ident, $a:ident, $i:tt)),+) => {
        impl<$($t: Respond),+> Respond for Mixed<($($t),+,)> {
            type Type = ($(<$t as Respond>::Type),+,);
        }

        impl<$($t),+, $($a),+> IntoResponse<Mixed<($($t),+,)>> for ($($a),+,)
        where
            $($t: Respond),+,
            $(<$t as Respond>::Type: Send + Sync),+,
            $($a: IntoResponse<$t>),+,
        {
            fn into_response(self) -> <Mixed<($($t),+,)> as Respond>::Type {
                ($(self.$i.into_response()),+,)
            }
        }

        impl<$($t),+, $($a),+> IntoOnceResponder<Mixed<($($t),+,)>> for ($($a),+,)
        where
            $($t: Respond),+,
            $(<$t as Respond>::Type: Send + Sync),+,
            $($a: IntoResponse<$t>),+,
        {
            fn into_once_responder<F: MockFn<Response = Mixed<($($t),+,)>>>(self) -> OutputResult<Responder> {
                let response = <Self as IntoResponse<Mixed<($($t),+,)>>>::into_response(self);
                Ok(Responder(DynResponder::new_cell::<F>(response)?))
            }
        }


        impl<$($t),+, $($a),+> IntoCloneResponder<Mixed<($($t),+,)>> for ($($a),+,)
        where
            $($t: Respond),+,
            $(<$t as Respond>::Type: Clone + Send + Sync),+,
            $($a: IntoCloneResponder<$t>),+,
        {
            fn into_clone_responder<F: MockFn<Response = Mixed<($($t),+,)>>>(self) -> OutputResult<Responder> {
                let response = <Self as IntoResponse<Mixed<($($t),+,)>>>::into_response(self);
                Ok(Responder(DynResponder::new_clone_cell::<F>(response)))
            }
        }

        impl<'u, $($t),+, $($a),+> Output<'u, Mixed<($($t),+,)>> for Mixed<($($a),+,)>
        where
            $($t: Respond),+,
            $($a: Output<'u, $t>),+,
        {
            type Type = ($(<$a as Output<'u, $t>>::Type),+,);

            fn from_response(
                response: <Mixed<($($t),+,)> as Respond>::Type,
                value_chain: &'u ValueChain,
            ) -> Self::Type {
                (
                    $(<$a as Output<'u, $t>>::from_response(response.$i, value_chain)),+,
                )
            }

            fn try_from_borrowed_response(
                response: &'u <Mixed<($($t),+,)> as Respond>::Type,
            ) -> OutputResult<Self::Type> {
                Ok((
                    $(<$a as Output<'u, $t>>::try_from_borrowed_response(&response.$i)?),+,
                ))
            }
        }
    };
}

mixed_tuples!((T0, A0, 0));
mixed_tuples!((T0, A0, 0), (T1, A1, 1));
mixed_tuples!((T0, A0, 0), (T1, A1, 1), (T2, A2, 2));
mixed_tuples!((T0, A0, 0), (T1, A1, 1), (T2, A2, 2), (T3, A3, 3));

// This can perhaps serve as the template for a macro that handles Mixed enums
mod mixed_poll {
    use super::*;
    use core::task::Poll;

    type Mix<T> = Mixed<Poll<T>>;

    impl<T> Respond for Mix<T>
    where
        Mixed<T>: Respond,
        <Mixed<T> as Respond>::Type: 'static + Send + Sync,
    {
        type Type = Poll<<Mixed<T> as Respond>::Type>;
    }

    impl<T0, T> IntoResponse<Mix<T>> for Poll<T0>
    where
        T0: IntoResponse<Mixed<T>>,
        Mixed<T>: Respond,
        <Mixed<T> as Respond>::Type: 'static + Send + Sync,
    {
        fn into_response(self) -> <Mix<T> as Respond>::Type {
            match self {
                Poll::Ready(value) => Poll::Ready(value.into_response()),
                Poll::Pending => Poll::Pending,
            }
        }
    }

    impl<T0, T> IntoOnceResponder<Mix<T>> for Poll<T0>
    where
        T0: IntoResponse<Mixed<T>>,
        Mixed<T>: Respond,
        <Mixed<T> as Respond>::Type: 'static + Send + Sync,
    {
        fn into_once_responder<F: MockFn<Response = Mix<T>>>(self) -> OutputResult<Responder> {
            match self {
                Poll::Ready(value) => Ok(Responder(DynResponder::new_cell::<F>(Poll::Ready(
                    value.into_response(),
                ))?)),
                Poll::Pending => Ok(Responder(DynResponder::new_cell::<F>(Poll::Pending)?)),
            }
        }
    }

    impl<T0, T> IntoCloneResponder<Mix<T>> for Poll<T0>
    where
        T0: IntoResponse<Mixed<T>>,
        Mixed<T>: Respond,
        <Mixed<T> as Respond>::Type: 'static + Send + Sync + Clone,
    {
        fn into_clone_responder<F: MockFn<Response = Mix<T>>>(self) -> OutputResult<Responder> {
            match self {
                Poll::Ready(value) => Ok(Responder(DynResponder::new_clone_cell::<F>(
                    Poll::Ready(value.into_response()),
                ))),
                Poll::Pending => Ok(Responder(DynResponder::new_cell::<F>(Poll::Pending)?)),
            }
        }
    }

    impl<'u, T, A> Output<'u, Mix<T>> for Mixed<Poll<A>>
    where
        Mixed<T>: Respond,
        <Mixed<T> as Respond>::Type: 'static + Send + Sync,
        Mixed<A>: Output<'u, Mixed<T>>,
    {
        type Type = Poll<<Mixed<A> as Output<'u, Mixed<T>>>::Type>;

        fn from_response(
            response: <Mix<T> as Respond>::Type,
            value_chain: &'u ValueChain,
        ) -> Self::Type {
            match response {
                Poll::Ready(value) => Poll::Ready(
                    <Mixed<A> as Output<'u, Mixed<T>>>::from_response(value, value_chain),
                ),
                Poll::Pending => Poll::Pending,
            }
        }

        fn try_from_borrowed_response(
            response: &'u <Mix<T> as Respond>::Type,
        ) -> OutputResult<Self::Type> {
            Ok(match response {
                Poll::Ready(value) => Poll::Ready(
                    <Mixed<A> as Output<'u, Mixed<T>>>::try_from_borrowed_response(value)?,
                ),
                Poll::Pending => Poll::Pending,
            })
        }
    }
}
