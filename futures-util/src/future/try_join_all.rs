//! Definition of the `TryJoinAll` combinator, waiting for all of a list of
//! futures to finish with either success or error.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::fmt;
use core::future::Future;
use core::iter::FromIterator;
use core::mem;
use core::pin::Pin;
use core::task::{Context, Poll};

use super::{assert_future, join_all, TryFuture, TryMaybeDone};

#[cfg(not(futures_no_atomic_cas))]
use crate::stream::{FuturesOrdered, TryCollect, TryStreamExt};

enum FinalState<E = ()> {
    Pending,
    AllDone,
    Error(E),
}

/// Future for the [`try_join_all`] function.
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct TryJoinAll<F>
where
    F: TryFuture,
{
    kind: TryJoinAllKind<F>,
}

enum TryJoinAllKind<F>
where
    F: TryFuture,
{
    Small {
        elems: Pin<Box<[TryMaybeDone<F>]>>,
    },
    #[cfg(not(futures_no_atomic_cas))]
    Big {
        fut: TryCollect<FuturesOrdered<F>, Vec<F::Ok>>,
    },
}

impl<F> fmt::Debug for TryJoinAll<F>
where
    F: TryFuture + fmt::Debug,
    F::Ok: fmt::Debug,
    F::Error: fmt::Debug,
    F::Output: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            TryJoinAllKind::Small { ref elems } => {
                f.debug_struct("TryJoinAll").field("elems", elems).finish()
            }
            #[cfg(not(futures_no_atomic_cas))]
            TryJoinAllKind::Big { ref fut, .. } => fmt::Debug::fmt(fut, f),
        }
    }
}

/// Creates a future which represents either a collection of the results of the
/// futures given or an error.
///
/// The returned future will drive execution for all of its underlying futures,
/// collecting the results into a destination `Vec<T>` in the same order as they
/// were provided.
///
/// If any future returns an error then all other futures will be canceled and
/// an error will be returned immediately. If all futures complete successfully,
/// however, then the returned future will succeed with a `Vec` of all the
/// successful results.
///
/// This function is only available when the `std` or `alloc` feature of this
/// library is activated, and it is activated by default.
///
/// # Examples
///
/// ```
/// # futures::executor::block_on(async {
/// use futures::future::{self, try_join_all};
///
/// let futures = vec![
///     future::ok::<u32, u32>(1),
///     future::ok::<u32, u32>(2),
///     future::ok::<u32, u32>(3),
/// ];
///
/// assert_eq!(try_join_all(futures).await, Ok(vec![1, 2, 3]));
///
/// let futures = vec![
///     future::ok::<u32, u32>(1),
///     future::err::<u32, u32>(2),
///     future::ok::<u32, u32>(3),
/// ];
///
/// assert_eq!(try_join_all(futures).await, Err(2));
/// # });
/// ```
pub fn try_join_all<I>(iter: I) -> TryJoinAll<I::Item>
where
    I: IntoIterator,
    I::Item: TryFuture
        + Future<Output = Result<<I::Item as TryFuture>::Ok, <I::Item as TryFuture>::Error>>,
{
    #[cfg(futures_no_atomic_cas)]
    {
        let elems = iter.into_iter().map(TryMaybeDone::Future).try_collect::<Box<[_]>>().into();
        let kind = TryJoinAllKind::Small { elems };
        assert_future::<Result<Vec<<I::Item as TryFuture>::Ok>, <I::Item as TryFuture>::Error>, _>(
            TryJoinAll { kind },
        )
    }
    #[cfg(not(futures_no_atomic_cas))]
    {
        let iter = iter.into_iter();
        let kind = match iter.size_hint().1 {
            None => TryJoinAllKind::Big { fut: iter.collect::<FuturesOrdered<_>>().try_collect() },
            Some(max) => {
                if max <= join_all::SMALL {
                    let elems = iter.map(TryMaybeDone::Future).collect::<Box<[_]>>().into();
                    TryJoinAllKind::Small { elems }
                } else {
                    TryJoinAllKind::Big { fut: iter.collect::<FuturesOrdered<_>>().try_collect() }
                }
            }
        };
        assert_future::<Result<Vec<<I::Item as TryFuture>::Ok>, <I::Item as TryFuture>::Error>, _>(
            TryJoinAll { kind },
        )
    }
}

impl<F> Future for TryJoinAll<F>
where
    F: TryFuture + Future<Output = Result<F::Ok, F::Error>>,
{
    type Output = Result<Vec<F::Ok>, F::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match &mut self.kind {
            TryJoinAllKind::Small { elems } => {
                let mut state = FinalState::AllDone;

                for elem in join_all::iter_pin_mut(elems.as_mut()) {
                    match elem.try_poll(cx) {
                        Poll::Pending => state = FinalState::Pending,
                        Poll::Ready(Ok(())) => {}
                        Poll::Ready(Err(e)) => {
                            state = FinalState::Error(e);
                            break;
                        }
                    }
                }

                match state {
                    FinalState::Pending => Poll::Pending,
                    FinalState::AllDone => {
                        let mut elems = mem::replace(elems, Box::pin([]));
                        let results = join_all::iter_pin_mut(elems.as_mut())
                            .map(|e| e.take_output().unwrap())
                            .collect();
                        Poll::Ready(Ok(results))
                    }
                    FinalState::Error(e) => {
                        let _ = mem::replace(elems, Box::pin([]));
                        Poll::Ready(Err(e))
                    }
                }
            }
            #[cfg(not(futures_no_atomic_cas))]
            TryJoinAllKind::Big { fut } => Pin::new(fut).poll(cx),
        }
    }
}

impl<F> FromIterator<F> for TryJoinAll<F>
where
    F: TryFuture + Future<Output = Result<F::Ok, F::Error>>,
{
    fn from_iter<T: IntoIterator<Item = F>>(iter: T) -> Self {
        try_join_all(iter)
    }
}
