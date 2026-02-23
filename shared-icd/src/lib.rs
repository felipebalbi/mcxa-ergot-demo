#![no_std]

use core::pin::pin;

use ergot::{NetStack, endpoint, interface_manager::profiles::null::Null};
use mutex::raw_impls::cs::CriticalSectionRawMutex;

pub type Network = NetStack<CriticalSectionRawMutex, Null>;

endpoint!(PingEndpoint, u8, u8, "ping");

pub async fn ping_service(net_stack: &'static Network) -> ! {
    let socket = net_stack
        .endpoints()
        .bounded_server::<PingEndpoint, 2>(None);
    let socket = pin!(socket);
    let mut hdl = socket.attach();

    loop {
        let _ = hdl.serve(async |packet| *packet).await;
    }
}
