use crate::JustBusError::ActorError;
use actix::{Actor, Addr, Context, Handler, MailboxError, Message};
use actix_web::{
    web, App, Error as ActixErr, HttpRequest, HttpResponse, HttpServer, Responder, ResponseError,
};
use futures::future::{ok as fut_ok, Either};
use lru_time_cache::LruCache;
use lta::bus::bus_arrival::ArrivalBusService;
use lta::r#async::{bus::get_arrival, lta_client::LTAClient, prelude::*};
use serde::Serialize;
use std::fmt::Formatter;
use std::{env::var, time::Duration};

type LruCacheU32 = LruCache<u32, TimingResult>;

struct CheckLru(u32);
struct AddLru(u32, TimingResult);

struct LruActor(LruCacheU32);

impl Message for CheckLru {
    type Result = Result<Option<Vec<ArrivalBusService>>, JustBusError>;
}

impl Message for AddLru {
    type Result = Result<TimingResult, JustBusError>;
}

impl Actor for LruActor {
    type Context = Context<Self>;

    fn started(&mut self, _: &mut Self::Context) {
        println!("LruActor started, spawning LruCache!");
    }

    fn stopped(&mut self, _: &mut Self::Context) {
        println!("LruActor stopped!");
    }
}

impl Handler<CheckLru> for LruActor {
    type Result = Result<Option<Vec<ArrivalBusService>>, JustBusError>;

    fn handle(&mut self, msg: CheckLru, _: &mut Self::Context) -> Self::Result {
        println!("LruActor CheckLru!");
        let data = self.0.peek(&msg.0).map(|u| u.data.clone());
        Ok(data)
    }
}

impl Handler<AddLru> for LruActor {
    type Result = Result<TimingResult, JustBusError>;

    fn handle(&mut self, msg: AddLru, _: &mut Self::Context) -> Self::Result {
        let data = msg.1.clone();
        println!("LruActor AddToLruPing");
        self.0.insert(msg.0, msg.1);
        Ok(data)
    }
}

#[derive(Debug)]
enum JustBusError {
    ClientError(lta::Error),
    ActorError(String),
}

impl From<MailboxError> for JustBusError {
    fn from(_: MailboxError) -> Self {
        ActorError("MailBoxError".to_string())
    }
}

impl From<()> for JustBusError {
    fn from(_: ()) -> Self {
        ActorError("MailBoxError".to_string())
    }
}

impl std::fmt::Display for JustBusError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Internal Server Error")
    }
}

impl ResponseError for JustBusError {
    fn error_response(&self) -> HttpResponse {
        HttpResponse::InternalServerError().finish()
    }
}
#[derive(Serialize, Clone)]
#[serde(rename_all(serialize = "PascalCase"))]
struct TimingResult {
    pub bus_stop_code: u32,
    pub data: Vec<ArrivalBusService>,
}

impl TimingResult {
    pub fn new(bus_stop_code: u32, data: Vec<ArrivalBusService>) -> Self {
        TimingResult {
            bus_stop_code,
            data,
        }
    }
}

impl Responder for TimingResult {
    type Error = ActixErr;
    type Future = Result<HttpResponse, ActixErr>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        let body = serde_json::to_string(&self.data)?;

        Ok(HttpResponse::Ok()
            .content_type("application/json")
            .body(body))
    }
}

fn get_timings(
    path: web::Path<u32>,
    client: web::Data<LTAClient>,
    lru_actor: web::Data<Addr<LruActor>>,
) -> impl Future<Item = HttpResponse, Error = JustBusError> {
    let bus_stop = path.into_inner();
    lru_actor
        .send(CheckLru(bus_stop))
        .from_err()
        .and_then(move |res| match res {
            Ok(data) => match data {
                Some(vec) => Either::A(fut_ok(
                    HttpResponse::Ok().json(TimingResult::new(bus_stop, vec.clone())),
                )),
                None => Either::B(
                    get_arrival(&client, bus_stop, None)
                        .map_err(JustBusError::ClientError)
                        .and_then(move |r| {
                            let data = r.services.clone();
                            lru_actor
                                .send(AddLru(bus_stop, TimingResult::new(bus_stop, data)))
                                .from_err()
                        })
                        .map(|f| match f {
                            Ok(t) => HttpResponse::Ok().json(t),
                            Err(e) => {
                                println!("{:?}", e);
                                HttpResponse::InternalServerError().finish()
                            }
                        }),
                ),
            },
            Err(_) => Either::A(fut_ok(HttpResponse::InternalServerError().finish())),
        })
}

fn main() {
    println!("Starting server @ 127.0.0.1:8080");
    let api_key = var("API_KEY").unwrap();
    let lta_client = LTAClient::with_api_key(api_key);
    let ttl = Duration::from_millis(1000 * 60);
    let sys = actix::System::new("LRU");

    HttpServer::new(move || {
        let lru_cache = LruCacheU32::with_expiry_duration(ttl);
        let lru_actor = LruActor(lru_cache).start();
        App::new()
            .route("/api/v1/timings/{bus_stop}", web::get().to_async(get_timings))
            .data(lta_client.clone())
            .data(lru_actor)
    })
    .bind("127.0.0.1:8080")
    .unwrap()
    .run()
    .unwrap();

    let _ = sys.run();
}
