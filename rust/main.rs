mod service_config;
mod service_host;
mod service_core;
#[cfg(test)]
mod service_tests;

fn main() {
    service_core::main_entry();
}
