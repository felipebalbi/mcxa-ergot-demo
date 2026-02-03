#![no_std]
#![no_main]

use core::pin::pin;

use embassy_executor::{Spawner, task};
use embassy_mcxa::bind_interrupts;
use embassy_mcxa::clocks::config::Div8;
use embassy_mcxa::config::Config;
use embassy_mcxa::gpio::{DriveStrength, Input, Level, Output, Pull, SlewRate};
use embassy_mcxa::i2c::Async;
use embassy_mcxa::i2c::controller::{self, InterruptHandler, Speed};
use embassy_mcxa::peripherals::LPI2C2;
use embassy_time::{Duration, WithTimeout};
use embedded_hal::digital::{InputPin, OutputPin};
use embedded_hal_async::delay::DelayNs;
use embedded_hal_async::digital::Wait;
use embedded_hal_async::i2c::I2c;
use ergot::{Address, NetStack, endpoint, interface_manager::profiles::null::Null, topic};
use mutex::raw_impls::cs::CriticalSectionRawMutex;
use tmp108::Tmp108;
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(
    struct Irqs {
        LPI2C2 => InterruptHandler<LPI2C2>;
    }
);

pub static STACK: NetStack<CriticalSectionRawMutex, Null> = NetStack::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut config = Config::default();
    config.clock_cfg.sirc.fro_lf_div = Div8::from_divisor(1);

    let p = embassy_mcxa::init(config);

    defmt::info!("Start");

    let btn = Input::new(p.P1_7, Pull::Disabled);

    let mut config = controller::Config::default();
    config.speed = Speed::Standard;
    let i2c = controller::I2c::new_async(p.LPI2C2, p.P1_9, p.P1_8, Irqs, config).unwrap();
    let tmp = Tmp108::new_with_a0_gnd(i2c);

    let red = Output::new(p.P3_18, Level::High, DriveStrength::Normal, SlewRate::Fast);
    let green = Output::new(p.P3_19, Level::High, DriveStrength::Normal, SlewRate::Fast);

    // Control LEDs
    spawner.spawn(led_server("RED", red).unwrap());
    spawner.spawn(led_server("GREEN", green).unwrap());

    // Listen to button presses
    spawner.spawn(press_listener(1).unwrap());
    spawner.spawn(press_listener(2).unwrap());
    spawner.spawn(press_listener(3).unwrap());

    // Listen to new temperature reports
    spawner.spawn(temperature_listener().unwrap());

    // Sample temperature sensor
    spawner.spawn(tmp108_worker("RED", tmp).unwrap());

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
async fn tmp108_worker(name: &'static str, tmp: Tmp108<controller::I2c<'static, Async>>) {
    tmp108_service(&STACK, name, tmp, embassy_time::Delay).await
}

#[task(pool_size = 2)]
async fn led_server(name: &'static str, led: Output<'static>) {
    led_service(&STACK, name, led).await
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

// LED service
endpoint!(LedEndpoint, bool, (), "led/set");

async fn led_service<O: OutputPin>(
    net_stack: &'static NetStack<CriticalSectionRawMutex, Null>,
    name: &'static str,
    mut led: O,
) -> ! {
    let socket = net_stack
        .endpoints()
        .bounded_server::<LedEndpoint, 2>(Some(name));
    let socket = pin!(socket);
    let mut hdl = socket.attach();
    loop {
        let _ = hdl
            .serve(async |on| {
                defmt::info!("{=str} set {=bool}", name, *on);
                if *on {
                    led.set_low().unwrap();
                } else {
                    led.set_high().unwrap();
                }
            })
            .await;
    }
}

// Temperature service

topic!(TemperatureTopic, f32, "temperature/latest");

async fn temperature_service(net_stack: &'static NetStack<CriticalSectionRawMutex, Null>) -> ! {
    let recv = net_stack
        .topics()
        .bounded_receiver::<TemperatureTopic, 3>(None);
    let recv = pin!(recv);
    let mut recv = recv.subscribe();

    loop {
        let msg = recv.recv().await;
        defmt::info!("Temperature {=f32}C", msg.t);
    }
}

async fn tmp108_service<I2C: I2c, DELAY: DelayNs>(
    net_stack: &'static NetStack<CriticalSectionRawMutex, Null>,
    name: &'static str,
    mut tmp: Tmp108<I2C>,
    mut delay: DELAY,
) -> ! {
    let client = net_stack
        .endpoints()
        .client::<LedEndpoint>(Address::unknown(), Some(name));
    loop {
        let temperature = tmp.temperature().await.unwrap();
        let _ = net_stack
            .topics()
            .broadcast::<TemperatureTopic>(&temperature, None);
        if temperature > 25.0 {
            client.request(&true).await.unwrap();
        } else if temperature < 22.0 {
            client.request(&false).await.unwrap();
        } else {
            // do nothing
        }

        delay.delay_ms(250).await;
    }
}

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
