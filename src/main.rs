#![no_std]
#![no_main]

use core::pin::pin;

use embassy_executor::{Spawner, task};
use embassy_mcxa::bind_interrupts;
use embassy_mcxa::clocks::config::Div8;
use embassy_mcxa::config::Config;
use embassy_mcxa::ctimer::CTimer;
use embassy_mcxa::ctimer::pwm::{DualPwm, Pwm};
use embassy_mcxa::gpio::{DriveStrength, Input, Level, Output, Pull, SlewRate};
use embassy_mcxa::i3c::controller;
use embassy_mcxa::i3c::{Async, InterruptHandler};
use embassy_mcxa::peripherals::I3C0;
use embassy_time::{Duration, WithTimeout};
use embedded_hal::digital::InputPin;
use embedded_hal_async::digital::Wait;
use ergot::{Address, NetStack, interface_manager::profiles::null::Null, topic};
use led_service::{LedEndpoint, led_service};
use mutex::raw_impls::cs::CriticalSectionRawMutex;
use pwm_service::pwm_service;
use shared_icd::Network;
use temperature_service::{temperature_service, tmp108_service};
use tmp108::Tmp108;
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(
    struct Irqs {
        I3C0 => InterruptHandler<I3C0>;
    }
);

static STACK: Network = NetStack::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut config = Config::default();
    config.clock_cfg.sirc.fro_lf_div = Div8::from_divisor(1);

    let p = embassy_mcxa::init(config);

    defmt::info!("Start");

    let btn = Input::new(p.P1_7, Pull::Disabled);

    let config = controller::Config::default();
    let i3c = controller::I3c::new_async(p.I3C0, p.P1_9, p.P1_8, Irqs, config).unwrap();
    let tmp = Tmp108::new_with_a0_gnd(i3c);

    let ctimer = CTimer::new(p.CTIMER2, Default::default()).unwrap();
    let pwm = DualPwm::new(
        ctimer,
        p.CTIMER2_CH0,
        p.CTIMER2_CH3,
        p.CTIMER2_CH2,
        p.P3_18,
        p.P3_21,
        Default::default(),
    )
    .unwrap();
    let (red, blue) = pwm.split();

    let green = Output::new(p.P3_19, Level::High, DriveStrength::Normal, SlewRate::Fast);

    // Control LEDs
    spawner.spawn(pwm_server(red, blue).unwrap());
    spawner.spawn(led_server("GREEN", green).unwrap());

    // Listen to button presses
    spawner.spawn(press_listener(1).unwrap());
    spawner.spawn(press_listener(2).unwrap());
    spawner.spawn(press_listener(3).unwrap());

    // Listen to new temperature reports
    spawner.spawn(temperature_listener().unwrap());

    // REVISIT: what's the correct way to synchronize these services?
    embassy_time::Timer::after_millis(100).await;

    // Sample temperature sensor
    spawner.spawn(tmp108_worker(tmp).unwrap());

    // Notify of button presses
    spawner.spawn(button_worker("GREEN", btn).unwrap());
}

#[task(pool_size = 3)]
async fn press_listener(idx: u8) {
    press_service(&STACK, idx).await
}

#[task]
async fn temperature_listener() {
    temperature_service(&STACK).await
}

#[task]
async fn tmp108_worker(tmp: Tmp108<controller::I3c<'static, Async>>) {
    tmp108_service(&STACK, tmp, embassy_time::Delay).await
}

#[task]
async fn led_server(name: &'static str, led: Output<'static>) {
    led_service(&STACK, name, led).await
}

#[task]
async fn pwm_server(red: Pwm<'static>, blue: Pwm<'static>) {
    pwm_service(&STACK, red, blue).await
}

#[task]
async fn button_worker(name: &'static str, btn: Input<'static>) {
    button_service(&STACK, name, btn).await
}

// ------------------------------------------------------------------------
//
// The following could be placed on a "services" crate. Or even split
// among several crates. As long as they know to declare their topics
// and endpoints accordingly.

// Button service

topic!(ButtonPressedTopic, u8, "button/press");

async fn button_service<I: InputPin + Wait>(
    net_stack: &'static NetStack<CriticalSectionRawMutex, Null>,
    name: &'static str,
    mut btn: I,
) -> ! {
    let client = net_stack
        .endpoints()
        .client::<LedEndpoint>(Address::unknown(), Some(name));
    loop {
        btn.wait_for_low().await.unwrap();
        let res = btn
            .wait_for_high()
            .with_timeout(Duration::from_millis(5))
            .await;
        if res.is_ok() {
            continue;
        }
        client.request(&true).await.unwrap();
        let _ = net_stack.topics().broadcast::<ButtonPressedTopic>(&1, None);
        btn.wait_for_high().await.unwrap();
        client.request(&false).await.unwrap();
    }
}

async fn press_service(net_stack: &'static NetStack<CriticalSectionRawMutex, Null>, idx: u8) -> ! {
    let recv = net_stack
        .topics()
        .bounded_receiver::<ButtonPressedTopic, 3>(None);
    let recv = pin!(recv);
    let mut recv = recv.subscribe();

    loop {
        let msg = recv.recv().await;
        defmt::info!("Listener #{=u8}, button {=u8} pressed", idx, msg.t);
    }
}
