//! Definition of the `Result` combinator

use core::pin::Pin;
use futures_core::future::{FusedFuture, Future};
use futures_core::task::{Context, Poll};
use pin_project_lite::pin_project;

pin_project! {
    /// A future representing either success or failure.
    ///
    /// Created by the [`From`] implementation for [`Result`](std::result::Result).
    ///
    /// # Examples
    ///
    /// ```
    /// # futures::executor::block_on(async {
    /// use futures::future::ResultFuture;
    ///
    /// let mut a: ResultFuture<_, _> = Ok(async { 123 }).into();
    /// assert_eq!(a.await, Ok(123));
    ///
    /// a = Err(()).into();
    /// assert_eq!(a.await, Err(()));
    /// # });
    /// ```
    #[derive(Debug, Clone)]
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub struct ResultFuture<F, E> {
        #[pin]
        inner: Result<F, Option<E>>,
    }
}

impl<F, E> Future for ResultFuture<F, E>
where
    F: Future,
{
    type Output = Result<F::Output, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: `get_unchecked_mut` is never used to move the `Result` inside `self`.
        // `x` is guaranteed to be pinned because it comes from `self` which is pinned.
        let result = unsafe {
            Pin::get_unchecked_mut(self.project().inner).as_mut().map(|x| Pin::new_unchecked(x))
        };

        match result {
            Ok(x) => x.poll(cx).map(Ok),
            Err(e) => Poll::Ready(Err(e.take().expect("polled `ResultFuture` after completion"))),
        }
    }
}

impl<F, E> FusedFuture for ResultFuture<F, E>
where
    F: FusedFuture,
{
    fn is_terminated(&self) -> bool {
        match &self.inner {
            Ok(x) => x.is_terminated(),
            Err(_) => true,
        }
    }
}

impl<F, E> From<Result<F, E>> for ResultFuture<F, E> {
    fn from(result: Result<F, E>) -> Self {
        Self { inner: result.map_err(Some) }
    }
}
