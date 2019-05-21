use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::error::Error as StdError;

use futures::IntoFuture;
use futures::{future, Future};
use hyper::body::Payload;
use hyper::{Body, Error as HyperError, Method, Request, Response};
use maplit::btreemap;
use typed_headers::{mime, ContentLength, ContentType, HeaderMapExt};

use edgelet_test_utils::{get_unused_tcp_port, run_tcp_server};

#[derive(Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
struct RequestPath(String);

#[derive(Clone, PartialEq, Eq, Hash)]
struct HttpMethod(Method);

impl Ord for HttpMethod {
    fn cmp(&self, other: &HttpMethod) -> Ordering {
        self.0.as_str().cmp(other.0.as_str())
    }
}

impl PartialOrd for HttpMethod {
    fn partial_cmp(&self, other: &HttpMethod) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

trait CloneableService: objekt::Clone {
    type ReqBody: Payload;
    type ResBody: Payload;
    type Error: Into<Box<StdError + Send + Sync>>;
    type Future: Future<Item = Response<Self::ResBody>, Error = Self::Error>;

    fn call(&mut self, req: Request<Self::ReqBody>) -> Self::Future;
}

objekt::clone_trait_object!(CloneableService<ReqBody = Body, ResBody = Body, Error = HyperError, Future = ResponseFuture> + Send);

type ResponseFuture = Box<dyn Future<Item = Response<Body>, Error = HyperError> + Send>;
type RequestHandler = Box<
    dyn CloneableService<
            ReqBody = Body,
            ResBody = Body,
            Error = HyperError,
            Future = ResponseFuture,
        > + Send,
>;

impl<T, F> CloneableService for T
where
    T: Fn(Request<Body>) -> F + Clone,
    F: IntoFuture<Item = Response<Body>, Error = HyperError>,
{
    type ReqBody = Body;
    type ResBody = Body;
    type Error = F::Error;
    type Future = F::Future;

    fn call(&mut self, req: Request<Self::ReqBody>) -> Self::Future {
        (self)(req).into_future()
    }
}

fn make_req_dispatcher(
    mut dispatch_table: BTreeMap<(HttpMethod, RequestPath), RequestHandler>,
    mut default_handler: RequestHandler,
) -> impl FnMut(Request<Body>) -> ResponseFuture + Clone {
    move |req: Request<Body>| {
        let key = (
            HttpMethod(req.method().clone()),
            RequestPath(req.uri().path().to_string()),
        );
        let handler = dispatch_table.get_mut(&key).unwrap_or(&mut default_handler);

        Box::new(handler.call(req))
    }
}

macro_rules! routes {
    ($($method:ident $path:expr => $handler:expr),+ $(,)*) => ({
        btreemap! {
            $((HttpMethod(Method::$method), RequestPath(From::from($path))) => Box::new($handler) as RequestHandler,)*
        }
    });
}

fn main() {
    let port = get_unused_tcp_port();

    let on_get_networks = |_| {
        let response = "{ \"greeting\": \"Hola amigo!\" }";
        let repsonse_len = response.len();

        let mut response = Response::new(response.into());
        response
            .headers_mut()
            .typed_insert(&ContentLength(repsonse_len as u64));
        response
            .headers_mut()
            .typed_insert(&ContentType(mime::APPLICATION_JSON));

        Box::new(future::ok(response)) as ResponseFuture
    };

    let on_create_network = |_| {
        let response = r###"{
            "Id": "12345",
            "Warnings": ""
        }"###
            .to_string();

        let response_len = response.len();

        let mut response = Response::new(response.into());
        response
            .headers_mut()
            .typed_insert(&ContentLength(response_len as u64));
        response
            .headers_mut()
            .typed_insert(&ContentType(mime::APPLICATION_JSON));
        Box::new(future::ok(response)) as ResponseFuture
    };

    let dispatch_table = routes!(
        GET "/networks" => on_get_networks,
        POST "/networks" => on_create_network,
    );

    let default_handler = |_| (Box::new(future::ok(Response::new("boo".into()))) as ResponseFuture);

    let dispatcher =
        make_req_dispatcher(dispatch_table, Box::new(default_handler) as RequestHandler);

    let server = run_tcp_server("127.0.0.1", port, dispatcher).map_err(|err| eprintln!("{}", err));

    println!("Listening at http://127.0.0.1:{}/", port);

    tokio::run(server);
}
