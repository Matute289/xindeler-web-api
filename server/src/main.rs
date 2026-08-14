#![forbid(unsafe_code)]

mod config;
mod error;
mod http;
mod state;
mod web;

fn main() {
    env_logger::init();
    config::initialize().expect("Invalid web-api server configuration");
    web::start();
}
