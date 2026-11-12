//! End-to-end tests: real drivers and workers talking TCP to the simulator.

use projectorhud::model::*;
use projectorhud::panasonic::PanasonicDriver;
use projectorhud::pjlink::PjLinkDriver;
use projectorhud::sim::{self, SimSpec};
use projectorhud::worker;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn fast_spec() -> SimSpec {
    SimSpec {
        name: "Test PJ".into(),
        warm_up: Duration::from_millis(300),
        cool_down: Duration::from_millis(300),
        ..SimSpec::default()
    }
}

fn poll(d: &mut dyn Driver, st: &mut ProjectorState) -> Result<(), DriverError> {
    d.poll(st, &|_| {})
}

#[test]
fn pjlink_full_cycle() {
    let sim = sim::spawn(fast_spec()).unwrap();
    let mut d = PjLinkDriver::new("127.0.0.1", sim.port, "", Log::default());
    let mut st = ProjectorState::default();

    poll(&mut d, &mut st).unwrap();
    assert_eq!(st.power, Power::Standby);
    assert_eq!(st.device_name, "Test PJ");
    assert_eq!(st.class, 2);
    assert_eq!(
        st.inputs[0],
        InputSource {
            code: "31".into(),
            name: "HDMI 1".into()
        }
    );
    assert_eq!(st.errors, Some(Errors::default()));
    assert_eq!(
        st.lamps,
        vec![Lamp {
            hours: 120,
            on: false
        }]
    );
    assert_eq!(st.video_mute, None, "mute state is unknown in standby");

    // Input switching is refused in standby and reported as an error.
    assert!(matches!(
        d.command(&Command::Input("32".into())),
        Err(DriverError::Protocol(_))
    ));

    d.command(&Command::PowerOn).unwrap();
    poll(&mut d, &mut st).unwrap();
    assert_eq!(st.power, Power::Warming);
    std::thread::sleep(Duration::from_millis(400));
    poll(&mut d, &mut st).unwrap();
    assert_eq!(st.power, Power::On);
    assert_eq!(st.input.as_deref(), Some("31"));
    assert_eq!(st.video_mute, Some(false));

    d.command(&Command::VideoMute(true)).unwrap();
    d.command(&Command::Input("32".into())).unwrap();
    d.command(&Command::Freeze(true)).unwrap();
    poll(&mut d, &mut st).unwrap();
    assert_eq!(st.video_mute, Some(true));
    assert_eq!(st.audio_mute, Some(false));
    assert_eq!(st.input_name().as_deref(), Some("HDMI 2"));
    assert_eq!(st.freeze, Some(true));

    d.command(&Command::AudioMute(true)).unwrap();
    d.command(&Command::VideoMute(false)).unwrap();
    poll(&mut d, &mut st).unwrap();
    assert_eq!((st.video_mute, st.audio_mute), (Some(false), Some(true)));

    d.command(&Command::PowerOff).unwrap();
    poll(&mut d, &mut st).unwrap();
    assert_eq!(st.power, Power::Cooling);
}

#[test]
fn pjlink_authentication() {
    let sim = sim::spawn(SimSpec {
        password: Some("secret".into()),
        ..fast_spec()
    })
    .unwrap();
    let mut st = ProjectorState::default();

    let mut good = PjLinkDriver::new("127.0.0.1", sim.port, "secret", Log::default());
    poll(&mut good, &mut st).unwrap();
    assert_eq!(st.power, Power::Standby);

    let mut bad = PjLinkDriver::new("127.0.0.1", sim.port, "wrong", Log::default());
    assert!(matches!(poll(&mut bad, &mut st), Err(DriverError::Auth(_))));

    let mut none = PjLinkDriver::new("127.0.0.1", sim.port, "", Log::default());
    assert!(matches!(
        poll(&mut none, &mut st),
        Err(DriverError::Auth(_))
    ));
}

#[test]
fn pjlink_class1_and_errors() {
    let sim = sim::spawn(SimSpec {
        class: 1,
        errors: [0, 0, 2, 0, 1, 0],
        powered: true,
        ..fast_spec()
    })
    .unwrap();
    let mut d = PjLinkDriver::new("127.0.0.1", sim.port, "", Log::default());
    let mut st = ProjectorState::default();
    poll(&mut d, &mut st).unwrap();
    assert_eq!(st.class, 1);
    assert_eq!(st.freeze, None);
    assert_eq!(st.severity(), Severity::Error);
    let issues: Vec<String> = st.issues().into_iter().map(|i| i.0).collect();
    assert_eq!(issues, vec!["Temperature error", "Filter warning"]);
    assert!(
        d.command(&Command::Freeze(true)).is_err(),
        "class 1 has no freeze"
    );
}

#[test]
fn pjlink_survives_dropped_connection() {
    let spec = SimSpec {
        port: 0,
        ..fast_spec()
    };
    let sim = sim::spawn(spec.clone()).unwrap();
    let port = sim.port;
    let mut d = PjLinkDriver::new("127.0.0.1", port, "", Log::default());
    let mut st = ProjectorState::default();
    poll(&mut d, &mut st).unwrap();

    // Projector goes away: polls fail cleanly.
    drop(sim);
    std::thread::sleep(Duration::from_millis(200));
    let deadline = Instant::now() + Duration::from_secs(40);
    while poll(&mut d, &mut st).is_ok() {
        assert!(
            Instant::now() < deadline,
            "driver never noticed the projector vanished"
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    // It comes back on the same port: the same driver recovers without help.
    let _sim = sim::spawn(SimSpec { port, ..spec }).unwrap();
    poll(&mut d, &mut st).unwrap();
    assert_eq!(st.power, Power::Standby);
}

#[test]
fn panasonic_native() {
    let sim = sim::spawn(SimSpec {
        ntcontrol_port: Some(0),
        password: Some("pw".into()),
        ..fast_spec()
    })
    .unwrap();
    let port = sim.ntcontrol_port.unwrap();
    let mut st = ProjectorState::default();

    let mut d = PanasonicDriver::new("127.0.0.1", port, "admin1", "pw", Log::default());
    poll(&mut d, &mut st).unwrap();
    assert_eq!(st.power, Power::Standby);
    d.command(&Command::PowerOn).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    d.command(&Command::VideoMute(true)).unwrap();
    poll(&mut d, &mut st).unwrap();
    assert_eq!(st.power, Power::On);
    assert_eq!(st.video_mute, Some(true));
    assert_eq!(st.lamps[0].hours, 120);

    let mut bad = PanasonicDriver::new("127.0.0.1", port, "admin1", "nope", Log::default());
    assert!(matches!(poll(&mut bad, &mut st), Err(DriverError::Auth(_))));
}

#[test]
fn worker_reports_offline_then_recovers() {
    // Reserve a port with nothing listening on it.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let cfg = ProjectorConfig {
        id: 1,
        name: "W".into(),
        host: "127.0.0.1".into(),
        port,
        protocol: Protocol::PjLink,
        username: String::new(),
        password: String::new(),
        group: None,
        demo: false,
    };
    let handle = worker::spawn(cfg, Arc::new(|| {}));
    let wait_for = |want: Link, secs: u64| {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while handle.snapshot().link != want {
            assert!(
                Instant::now() < deadline,
                "never reached {want:?}; at {:?}",
                handle.snapshot().link
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    wait_for(Link::Offline, 5);
    assert!(handle.snapshot().link_error.is_some());

    let _sim = sim::spawn(SimSpec {
        port,
        ..fast_spec()
    })
    .unwrap();
    wait_for(Link::Online, 30);

    handle.send(Command::PowerOn);
    let deadline = Instant::now() + Duration::from_secs(10);
    while handle.snapshot().power != Power::On {
        assert!(Instant::now() < deadline, "power never came on");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn shutter_falls_back_to_combined_av_mute() {
    // A projector that only accepts AVMT 30/31 and does not say it is a Panasonic.
    let sim = sim::spawn(SimSpec {
        av_mute_only: true,
        powered: true,
        ..fast_spec()
    })
    .unwrap();
    let mut d = PjLinkDriver::new("127.0.0.1", sim.port, "", Log::default());
    let mut st = ProjectorState::default();
    poll(&mut d, &mut st).unwrap();
    assert_eq!(st.video_mute, Some(false));

    d.command(&Command::VideoMute(true)).unwrap();
    poll(&mut d, &mut st).unwrap();
    assert_eq!(st.video_mute, Some(true));
    assert_eq!(
        st.audio_mute, None,
        "no separate audio mute on this projector"
    );
    assert!(d.command(&Command::AudioMute(true)).is_err());

    d.command(&Command::VideoMute(false)).unwrap();
    poll(&mut d, &mut st).unwrap();
    assert_eq!(st.video_mute, Some(false));
}

#[test]
fn panasonic_shutter_uses_av_mute_from_the_start() {
    let sim = sim::spawn(SimSpec {
        manufacturer: "Panasonic".into(),
        av_mute_only: true,
        powered: true,
        ..fast_spec()
    })
    .unwrap();
    let log = Log::default();
    let mut d = PjLinkDriver::new("127.0.0.1", sim.port, "", log.clone());
    let mut st = ProjectorState::default();
    poll(&mut d, &mut st).unwrap();
    assert_eq!(st.audio_mute, None);
    d.command(&Command::VideoMute(true)).unwrap();
    let sent: Vec<String> = log.snapshot().into_iter().map(|e| e.text).collect();
    assert!(sent.iter().any(|l| l == "%1AVMT 31"));
    assert!(
        !sent.iter().any(|l| l == "%1AVMT 11"),
        "never sends the form Panasonic refuses"
    );
}
