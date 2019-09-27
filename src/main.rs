#[macro_use]
extern crate lazy_static;
extern crate jemallocator;
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


#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

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

fn get_timings(
    path: web::Path<u32>,
) -> impl Future<Item = HttpResponse, Error = JustBusError> {
    let inner_path = path.into_inner();
    let inner = LRU.read();
    let in_lru = inner.peek(&inner_path);

    match in_lru {
        Some(data) => {
            Either::A(fut_ok(
                HttpResponse::Ok().json(TimingResult::new(inner_path, data.clone().data)),
            ))
        }
        None => {
            println!(
                "Fresh data from LTA. client_ptr: {:p}, cache_ptr: {:p}",
                &CLIENT, &LRU
            );
            Either::B(
                get_arrival(&CLIENT, inner_path, None)
                    .then(move |r| {
                        r.map(|r| {
                            let bus_stop = inner_path;
                            let data = r.services.clone();
                            let mut lru_w = LRU.write();
                            lru_w.insert(bus_stop, TimingResult::new(bus_stop, data));
                            (r, bus_stop)
                        })
                    })
                    .map(|f| HttpResponse::Ok().json(TimingResult::new(f.1, f.0.services)))
                    .map_err(|e| JustBusError::ClientError(e)),
            )
        }
    }
}

type LruCacheU32 = LruCache<u32, TimingResult>;


lazy_static! {
    static ref CLIENT: LTAClient = {
        let api_key = var("API_KEY").unwrap();
        LTAClient::with_api_key(api_key)
    };

    static ref LRU: RwLock<LruCacheU32> = {
        let ttl = Duration::from_millis(1000 * 60);
        RwLock::new(LruCacheU32::with_expiry_duration(ttl))
    };
}

fn main() {
    println!("Starting server @ 127.0.0.1:8080");
    HttpServer::new(move || {
        App::new()
            .route(
                "/api/v1/timings/{bus_stop}",
                web::get().to_async(get_timings),
            )
    })
    .bind("127.0.0.1:8080")
    .unwrap()
    .run()
    .unwrap();
}
