use crate::collect_body;
use futures::future::Map;
use futures::FutureExt;
use http::Response;
use http_body::Body;
use http_body_util::Collected;
use serde::Deserialize;
use std::marker::PhantomData;
use std::task::{Context, Poll};
use tower::ServiceBuilder;

#[derive(Debug, thiserror::Error)]
pub enum Error<S, B> {
    #[error(transparent)]
    Service(S),
    #[error(transparent)]
    Body(B),
    #[error(transparent)]
    Json(serde_json::Error),
}

#[derive(Clone)]
pub struct Service<S, T> {
    inner: collect_body::Service<S>,
    _phantom: PhantomData<fn() -> T>,
}

impl<S, Request, B, T> tower::Service<Request> for Service<S, T>
where
    S: tower::Service<Request, Response = Response<B>>,
    B: Body,
    T: for<'de> Deserialize<'de>,
{
    type Response = Response<T>;
    type Error = Error<S::Error, B::Error>;
    type Future = Future<S, Request, B, T>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(map_err)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let f = self.inner.call(request);
        f.map(|response| {
            let response = response.map_err(map_err)?;
            let (parts, body) = response.into_parts();
            let body = serde_json::from_slice(&body.to_bytes()).map_err(Error::Json)?;
            let response = Response::from_parts(parts, body);
            Ok(response)
        })
    }
}

pub type Future<S, Request, B, T> = Map<
    <collect_body::Service<S> as tower::Service<Request>>::Future,
    fn(
        Result<
            Response<Collected<<B as Body>::Data>>,
            collect_body::Error<<S as tower::Service<Request>>::Error, <B as Body>::Error>,
        >,
    )
        -> Result<Response<T>, Error<<S as tower::Service<Request>>::Error, <B as Body>::Error>>,
>;

fn map_err<S, B>(e: collect_body::Error<S, B>) -> Error<S, B> {
    match e {
        collect_body::Error::Service(e) => Error::Service(e),
        collect_body::Error::Body(e) => Error::Body(e),
    }
}

pub struct Layer<T> {
    _phantom: PhantomData<fn() -> T>,
}

impl<T> Clone for Layer<T> {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl<T> Default for Layer<T> {
    fn default() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }
}

impl<S, T> tower::Layer<S> for Layer<T> {
    type Service = Service<S, T>;

    fn layer(&self, inner: S) -> Self::Service {
        let inner = ServiceBuilder::new()
            .layer(collect_body::Layer::default())
            .service(inner);
        Service {
            inner,
            _phantom: PhantomData,
        }
    }
}
