use actix::{Actor, Context, Handler, Message, Addr};
use actix_web::{
    web, App, Error as ActixErr, HttpRequest, HttpResponse, HttpServer, Responder, ResponseError,
};
use futures::future::{ok as fut_ok, Either};
use lru_time_cache::LruCache;
use lta::bus::bus_arrival::ArrivalBusService;
use lta::r#async::{bus::get_arrival, lta_client::LTAClient, prelude::*};
use parking_lot::RwLock;
use serde::Serialize;
use std::fmt::Formatter;
use std::{env::var, time::Duration};

type LruCacheU32 = LruCache<u32, TimingResult>;

enum LruMessages {
    CheckLru(u32),
    AddLru(u32, TimingResult)
}

struct LruActor(LruCacheU32);

impl Message for LruMessages::CheckLru {
    type Result = Result<Option<Vec<ArrivalBusService>>, std::io::Error>;
}

impl Message for LruMessages::AddLru {
    type Result = Result<(), std::io::Error>;
}

impl Actor for LruActor {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        println!("LruActor started, spawning LruCache!");
    }

    fn stopped(&mut self, ctx: &mut Self::Context) {
        println!("LruActor stopped!");
    }
}

impl Handler<LruMessages::CheckLru> for LruActor {
    type Result = Result<Option<Vec<ArrivalBusService>>, std::io::Error>;

    fn handle(&mut self, msg: LruMessages::CheckLru, ctx: &mut Self::Context) -> Self::Result {
        println!("LruActor CheckLru !");
        let data = self.0.peek(&msg.0).map(|u| u.data.clone());
        Ok(data)
    }
}

impl Handler<LruMessages::AddLru> for LruActor {
    type Result = Result<(), std::io::Error>;

    fn handle(&mut self, msg: LruMessages::AddLru, ctx: &mut Self::Context) -> Self::Result {
        println!("LruActor AddToLruPing!");
        self.0.insert(msg.0, msg.1);
        Ok(())
    }
}

#[derive(Clone)]
struct LruState {
    pub lru: LruCache<u32, TimingResult>,
    pub client: LTAClient,
}

#[derive(Debug)]
enum JustBusError {
    ClientError(lta::Error),
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

impl LruState {
    pub fn new(lru: LruCache<u32, TimingResult>, client: LTAClient) -> Self {
        LruState { lru, client }
    }
}

fn get_timings(
    client: web::Data<LTAClient>,
    lru_actor: web::Data<Addr<LruActor>>
) -> impl Future<Item = HttpResponse, Error = JustBusError> {

    let msg_res_fut = lru_actor.send(LruMessages::CheckLru(83139));



    match msg_res {
        Some(data) => {
            //            println!("Taking from LRU");
            Either::A(fut_ok(
                HttpResponse::Ok().json(TimingResult::new(83139, data.clone().data)),
            ))
        }
        None => {
            println!(
                "Fresh data from LTA on Thread: {:?}",
                std::thread::current().id()
            );
            Either::B(
                get_arrival(&client, 83139, None)
                    .then(move |r| {
                        r.map(|r| {
                            let data = r.services.clone();
                            let mut lru_w = lru_state_2.write();
                            lru_w.lru.insert(83139, TimingResult::new(83139, data));
                            r
                        })
                    })
                    .map(|f| HttpResponse::Ok().json(TimingResult::new(83139, f.services)))
                    .map_err(|e| JustBusError::ClientError(e)),
            )
        }
    }
}

fn main() {
    println!("Starting server @ 127.0.0.1:8080");
    let api_key = var("API_KEY").unwrap();
    let ttl = Duration::from_millis(1000 * 15);
    let lru_cache = LruCacheU32::with_expiry_duration(ttl);
    let lru_actor = LruActor(lru_cache).start();

    HttpServer::new(move || {
        let api_key = var("API_KEY").unwrap();
        let lta_client = LTAClient::with_api_key(api_key);
        App::new()
            .route("/api/v1/timings", web::get().to_async(get_timings))
            .data(lta_client)
            .data(lru_actor)
    })
    .bind("127.0.0.1:8080")
    .unwrap()
    .run()
    .unwrap();
}
