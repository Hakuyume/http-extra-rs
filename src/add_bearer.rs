use futures::future::BoxFuture;
use futures::FutureExt;
use futures::TryFutureExt;
use headers::authorization::{Bearer, InvalidBearerToken};
use headers::{Authorization, HeaderMapExt};
use http::Request;
use std::convert::Infallible;
use std::env;
use std::ffi::OsStr;
use std::future;
use std::io;
use std::mem;
#[cfg(feature = "tokio-fs")]
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{ready, Context, Poll};

#[derive(Debug, thiserror::Error)]
pub enum Error<S> {
    #[error(transparent)]
    Env(env::VarError),
    #[error(transparent)]
    Io(io::Error),
    #[error(transparent)]
    InvalidBearerToken(InvalidBearerToken),
    #[error(transparent)]
    Service(S),
}

#[derive(Clone)]
pub struct Service<S> {
    inner: S,
    source: Source,
}

#[derive(Clone)]
enum Source {
    Env(Arc<OsStr>),
    #[cfg(feature = "tokio-fs")]
    File(Arc<Path>),
    Token(Authorization<Bearer>),
}

impl<S, B> tower::Service<Request<B>> for Service<S>
where
    S: Clone + tower::Service<Request<B>>,
{
    type Response = S::Response;
    type Error = Error<S::Error>;
    type Future = Future<S, B>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Error::Service)
    }

    fn call(&mut self, request: Request<B>) -> Self::Future {
        let inner = self.inner.clone();
        let inner = mem::replace(&mut self.inner, inner);
        let f = match self.source.clone() {
            Source::Env(key) => futures::future::lazy(move |_| env::var(key))
                .map(|token| {
                    Authorization::bearer(&token.map_err(Error::Env)?)
                        .map_err(Error::InvalidBearerToken)
                })
                .boxed(),
            #[cfg(feature = "tokio-fs")]
            Source::File(path) => tokio::fs::read_to_string(path)
                .map(|token| {
                    Authorization::bearer(&token.map_err(Error::Io)?)
                        .map_err(Error::InvalidBearerToken)
                })
                .boxed(),
            Source::Token(header) => future::ready(Ok(header)).boxed(),
        };
        Future(State::S0 {
            f,
            inner,
            request: Some(request),
        })
    }
}

#[pin_project::pin_project]
pub struct Future<S, B>(#[pin] State<S, B>)
where
    S: tower::Service<Request<B>>;

#[pin_project::pin_project(project = StateProj)]
enum State<S, B>
where
    S: tower::Service<Request<B>>,
{
    S0 {
        #[pin]
        f: BoxFuture<'static, Result<Authorization<Bearer>, Error<Infallible>>>,
        inner: S,
        request: Option<Request<B>>,
    },
    S1 {
        #[pin]
        f: S::Future,
    },
}

impl<S, B> future::Future for Future<S, B>
where
    S: tower::Service<Request<B>>,
{
    type Output = Result<S::Response, Error<S::Error>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        loop {
            match this.0.as_mut().project() {
                StateProj::S0 { f, inner, request } => {
                    let header = ready!(f.poll(cx)).map_err(|e| match e {
                        Error::Env(e) => Error::Env(e),
                        Error::Io(e) => Error::Io(e),
                        Error::InvalidBearerToken(e) => Error::InvalidBearerToken(e),
                    })?;
                    let mut request = request.take().unwrap();
                    request.headers_mut().typed_insert(header);
                    let f = inner.call(request);
                    this.0.set(State::S1 { f });
                }
                StateProj::S1 { f } => {
                    let response = ready!(f.poll(cx)).map_err(Error::Service)?;
                    break Poll::Ready(Ok(response));
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct Layer {
    source: Source,
}

impl<S> tower::Layer<S> for Layer {
    type Service = Service<S>;

    fn layer(&self, inner: S) -> Self::Service {
        Service {
            inner,
            source: self.source.clone(),
        }
    }
}

impl Layer {
    pub fn from_env<K>(key: K) -> Self
    where
        K: Into<Arc<OsStr>>,
    {
        Self {
            source: Source::Env(key.into()),
        }
    }

    #[cfg(feature = "tokio-fs")]
    pub fn from_file<P>(path: P) -> Self
    where
        P: Into<Arc<Path>>,
    {
        Self {
            source: Source::File(path.into()),
        }
    }

    pub fn from_token(token: &str) -> Result<Self, InvalidBearerToken> {
        Ok(Self {
            source: Source::Token(Authorization::bearer(token)?),
        })
    }
}
