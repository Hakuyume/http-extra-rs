use bytes::Bytes;
use http::Response;
use http_body::Body;
use http_body_util::combinators::Collect;
use http_body_util::BodyExt;
use std::future;
use std::pin::Pin;
use std::task::{ready, Context, Poll};

#[derive(Debug, thiserror::Error)]
pub enum Error<S, B> {
    #[error(transparent)]
    Service(S),
    #[error(transparent)]
    Body(B),
}

#[derive(Clone)]
pub struct Service<S> {
    inner: S,
}

impl<S, Request, B> tower::Service<Request> for Service<S>
where
    S: tower::Service<Request, Response = Response<B>>,
    B: Body,
{
    type Response = Response<Bytes>;
    type Error = Error<S::Error, B::Error>;
    type Future = Future<S, Request, B>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Error::Service)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let f = self.inner.call(request);
        Future(State::S0 { f })
    }
}

#[pin_project::pin_project]
pub struct Future<S, Request, B>(#[pin] State<S, Request, B>)
where
    S: tower::Service<Request, Response = Response<B>>,
    B: Body;

#[allow(clippy::large_enum_variant)]
#[pin_project::pin_project(project = StateProj)]
enum State<S, Request, B>
where
    S: tower::Service<Request, Response = Response<B>>,
    B: Body,
{
    S0 {
        #[pin]
        f: S::Future,
    },
    S1 {
        #[pin]
        f: Collect<B>,
        parts: Option<http::response::Parts>,
    },
}

impl<S, Request, B> future::Future for Future<S, Request, B>
where
    S: tower::Service<Request, Response = Response<B>>,
    B: Body,
{
    type Output = Result<Response<Bytes>, Error<S::Error, B::Error>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        loop {
            match this.0.as_mut().project() {
                StateProj::S0 { f } => {
                    let response = ready!(f.poll(cx)).map_err(Error::Service)?;
                    let (parts, body) = response.into_parts();
                    let f = body.collect();
                    this.0.set(State::S1 {
                        f,
                        parts: Some(parts),
                    });
                }
                StateProj::S1 { f, parts } => {
                    let body = ready!(f.poll(cx)).map_err(Error::Body)?;
                    let parts = parts.take().unwrap();
                    let response = Response::from_parts(parts, body.to_bytes());
                    break Poll::Ready(Ok(response));
                }
            }
        }
    }
}

#[derive(Clone, Default)]
pub struct Layer {}

impl<S> tower::Layer<S> for Layer {
    type Service = Service<S>;

    fn layer(&self, inner: S) -> Self::Service {
        Service { inner }
    }
}
