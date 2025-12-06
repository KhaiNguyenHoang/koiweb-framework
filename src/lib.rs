//! A micro web framework for Rust, inspired by Express.js.
//!
//! This library provides a simple and flexible way to build web applications
//! with a focus on ease of use and performance.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use hyper::service::service_fn;
use hyper::{
    Body, HeaderMap, Method, Request as HyperRequest, Response as HyperResponse,
    Server as HyperServer, StatusCode,
    body::to_bytes,
    header::{HeaderName, HeaderValue},
};

use thiserror::Error;

// -----------------------------
// 1. ADVANCED ERROR HANDLING
// -----------------------------

/// Custom error types for the application.
#[derive(Error, Debug)]
pub enum AppError {
    /// Resource not found error.
    #[error("Not Found")]
    NotFound,
    /// Internal server error with a detailed message.
    #[error("Internal Server Error: {0}")]
    Internal(String),
    /// Bad request error with a detailed message.
    #[error("Bad Request: {0}")]
    BadRequest(String),
    /// Too many requests error, typically for rate limiting.
    #[error("Too Many Requests")]
    TooManyRequests,
    /// I/O error wrapper.
    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),
    /// Serialization/Deserialization error wrapper.
    #[error("Serialization Error: {0}")]
    Serde(#[from] serde_json::Error),
}

impl AppError {
    /// Converts an `AppError` into an HTTP `Response`.
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            AppError::NotFound => (StatusCode::NOT_FOUND, "Not Found".to_string()),
            AppError::Internal(e) => (StatusCode::INTERNAL_SERVER_ERROR, e),
            AppError::BadRequest(e) => (StatusCode::BAD_REQUEST, e),
            AppError::TooManyRequests => (
                StatusCode::TOO_MANY_REQUESTS,
                "Rate limit exceeded".to_string(),
            ),
            AppError::Io(_) => (StatusCode::INTERNAL_SERVER_ERROR, "IO Error".to_string()),
            AppError::Serde(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Serialization Error".to_string(),
            ),
        };
        Response::new(status, msg)
    }
}

/// A type alias for `Result` that defaults to `AppError` for the error type.
pub type AppResult = Result<Response, AppError>;

// -----------------------------
// Core types (Refined)
// -----------------------------

/// Represents an incoming HTTP request.
pub struct Request {
    pub method: Method,
    pub path: String,
    pub headers: HeaderMap,
    pub body: Body,
    pub params: HashMap<String, String>,
    pub remote_addr: Option<SocketAddr>,
}

impl Request {
    /// Reads the entire request body into a `Bytes` buffer.
    pub async fn body_bytes(self) -> Result<Bytes, AppError> {
        to_bytes(self.body)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to read body: {}", e)))
    }

    /// Reads the request body and attempts to deserialize it from JSON into the specified type `T`.
    pub async fn body_json<T: serde::de::DeserializeOwned>(self) -> Result<T, AppError> {
        let bytes = self.body_bytes().await?;
        serde_json::from_slice(&bytes).map_err(AppError::Serde)
    }
}

// Represents an outgoing HTTP response.
pub struct Response {
    inner: HyperResponse<Body>,
}

impl Response {
    // Creates a new `Response` with the given status code and body.
    pub fn new(status: StatusCode, body: impl Into<Body>) -> Self {
        Response {
            inner: HyperResponse::builder()
                .status(status)
                .body(body.into())
                .unwrap(),
        }
    }

    /// Creates an OK (200) JSON response.
    /// The provided value `T` will be serialized to JSON and set as the response body.
    pub fn ok_json<T: serde::Serialize>(value: &T) -> AppResult {
        let body = serde_json::to_string(value)?;
        let mut res = Response::new(StatusCode::OK, Body::from(body));
        res.set_header("Content-Type", "application/json");
        Ok(res)
    }

    /// Consumes the `Response` and returns the underlying `hyper::Response`.
    pub fn into_inner(self) -> HyperResponse<Body> {
        self.inner
    }

    /// Sets a header on the response.
    ///
    /// If the header name is invalid, the operation is a no-op.
    pub fn set_header(&mut self, k: &str, v: &str) {
        let Ok(k) = HeaderName::from_bytes(k.as_bytes()) else {
            return;
        };
        if let Ok(val) = HeaderValue::from_str(v) {
            self.inner.headers_mut().insert(k, val);
        }
    }
}

/// A type alias for a pinned, boxed future that resolves to an `AppResult`.
pub type HandlerFuture = std::pin::Pin<Box<dyn futures::Future<Output = AppResult> + Send>>;
/// A type alias for a dynamic handler function.
pub type DynHandler = dyn Fn(Request) -> HandlerFuture + Send + Sync + 'static;

/// Boxes an async handler function into a `Box<DynHandler>`.
pub fn boxed_handler<F, Fut>(f: F) -> Box<DynHandler>
where
    F: Fn(Request) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = AppResult> + Send + 'static,
{
    Box::new(move |req| Box::pin(f(req)))
}

// Router

/// The core router that dispatches requests to the appropriate handlers.
pub struct Router {
    routes: HashMap<(Method, String), Arc<DynHandler>>,
    dynamic_routes: Vec<(Method, String, Arc<DynHandler>)>,
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

impl Router {
    /// Creates a new, empty `Router`.
    pub fn new() -> Self {
        Self {
            routes: HashMap::new(),
            dynamic_routes: Vec::new(),
        }
    }

    /// Adds a new route to the router.
    ///
    /// If the path contains a colon (`:`) it is considered a dynamic route.
    pub fn add<H, F>(&mut self, method: Method, path: &str, handler: H)
    where
        H: Fn(Request) -> F + Send + Sync + 'static,
        F: std::future::Future<Output = AppResult> + Send + 'static,
    {
        let handler = Arc::new(boxed_handler(handler));
        if path.contains(':') {
            self.dynamic_routes
                .push((method, path.to_string(), handler));
        } else {
            self.routes.insert((method, path.to_string()), handler);
        }
    }

    /// Resolves a request to a handler, if one matches.
    ///
    /// For dynamic routes, it extracts path parameters and stores them in `req.params`.
    pub fn resolve(&self, req: &mut Request) -> Option<Arc<DynHandler>> {
        if let Some(handler) = self.routes.get(&(req.method.clone(), req.path.clone())) {
            return Some(handler.clone());
        }

        for (method, pattern, handler) in &self.dynamic_routes {
            if method != &req.method {
                continue;
            }

            let pat_parts: Vec<&str> = pattern.split('/').collect();
            let path_parts: Vec<&str> = req.path.split('/').collect();

            if pat_parts.len() != path_parts.len() {
                continue;
            }

            let mut params = HashMap::new();
            let mut matched = true;

            for (i, part) in pat_parts.iter().enumerate() {
                if let Some(key) = part.strip_prefix(':') {
                    params.insert(key.to_string(), path_parts[i].to_string());
                } else if part != &path_parts[i] {
                    matched = false;
                    break;
                }
            }

            if matched {
                req.params = params;
                return Some(handler.clone());
            }
        }
        None
    }
}

/// A builder for grouping routes under a common prefix.
///
/// This allows for defining routes like `/api/v1/users` and `/api/v1/products`
/// by creating a group for `/api/v1`.
pub struct RouteGroup<'a> {
    prefix: String,
    app: &'a mut App,
}

impl<'a> RouteGroup<'a> {
    /// Adds a route to the current group.
    /// The `path` provided here will be prefixed with the group's prefix.
    ///
    /// Returns `&mut Self` for chaining.
    pub fn route<H, F>(&mut self, method: Method, path: &str, handler: H) -> &mut Self
    where
        H: Fn(Request) -> F + Send + Sync + 'static,
        F: std::future::Future<Output = AppResult> + Send + 'static,
    {
        let path = if path == "/" {
            self.prefix.clone()
        } else {
            format!("{}{}", self.prefix, path)
        };
        self.app.route(method, &path, handler);
        self
    }
}

// -----------------------------
// Middleware
// -----------------------------

/// A type alias for a middleware function.
///
/// Middleware functions have access to the incoming `Request` and can
/// either produce a `Response` or call the `next` handler in the chain.
pub type Middleware = dyn Fn(Request, Arc<DynHandler>) -> HandlerFuture + Send + Sync + 'static;

/// Chains multiple middleware functions together.
///
/// The middleware are executed in the order they are provided, with the
/// `final_handler` being called last.
pub fn chain_middleware(
    middlewares: &[Arc<Middleware>],
    final_handler: Arc<DynHandler>,
) -> Arc<DynHandler> {
    middlewares.iter().rfold(final_handler, |next, mw| {
        let mw = mw.clone();
        Arc::new(boxed_handler(move |req| {
            let next = next.clone();
            mw(req, next)
        }))
    })
}

// -----------------------------
// App Glue Code
// -----------------------------

/// The main application struct that holds the router and middleware.
///
/// This is the entry point for defining routes and starting the server.
pub struct App {
    router: Arc<Router>,
    middlewares: Vec<Arc<Middleware>>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    /// Creates a new `App` instance with an empty router and no middleware.
    pub fn new() -> Self {
        Self {
            router: Arc::new(Router::new()),
            middlewares: vec![],
        }
    }

    /// Adds a middleware to the application.
    ///
    /// Middlewares are executed in the order they are added.
    pub fn with_middleware(mut self, mw: Box<Middleware>) -> Self {
        self.middlewares.push(Arc::from(mw));
        self
    }

    /// Adds a new route to the application's router.
    ///
    /// This is a convenience method that delegates to the internal `Router`.
    pub fn route<H, F>(&mut self, method: Method, path: &str, handler: H)
    where
        H: Fn(Request) -> F + Send + Sync + 'static,
        F: std::future::Future<Output = AppResult> + Send + 'static,
    {
        Arc::get_mut(&mut self.router)
            .unwrap()
            .add(method, path, handler);
    }

    /// Creates a new `RouteGroup` for defining routes under a common prefix.
    pub fn group(&mut self, prefix: &str) -> RouteGroup<'_> {
        RouteGroup {
            prefix: prefix.to_string(),
            app: self,
        }
    }

    /// Starts the HTTP server and listens for incoming requests.
    pub async fn run(self, addr: &str, static_router: Option<Router>) -> std::io::Result<()> {
        let addr: SocketAddr = addr.parse().expect("Invalid address format");
        let router = self.router;
        let static_router = static_router.map(Arc::new);
        let middlewares = Arc::new(self.middlewares);

        let make_svc =
            hyper::service::make_service_fn(move |conn: &hyper::server::conn::AddrStream| {
                let remote_addr = Some(conn.remote_addr());
                let router = router.clone();
                let static_router = static_router.clone();
                let middlewares = middlewares.clone();

                async move {
                    Ok::<_, Infallible>(service_fn(move |req: HyperRequest<Body>| {
                        let router = router.clone();
                        let static_router = static_router.clone();
                        let middlewares = middlewares.clone();

                        async move {
                            let (parts, body) = req.into_parts();
                            let mut request = Request {
                                method: parts.method,
                                path: parts.uri.path().to_string(),
                                headers: parts.headers,
                                body,
                                params: HashMap::new(),
                                remote_addr,
                            };

                            let handler = router.resolve(&mut request).or_else(|| {
                                static_router.as_ref().and_then(|r| r.resolve(&mut request))
                            });

                            let result = match handler {
                                Some(handler) => {
                                    let chain = chain_middleware(&middlewares, handler);
                                    chain(request).await
                                }
                                None => Err(AppError::NotFound),
                            };

                            let response = match result {
                                Ok(res) => res.into_inner(),
                                Err(err) => err.into_response().into_inner(),
                            };

                            Ok::<_, Infallible>(response)
                        }
                    }))
                }
            });

        let server = HyperServer::bind(&addr).serve(make_svc);
        println!("Server upgraded running at http://{}", addr);
        server.await.map_err(std::io::Error::other)
    }
}
