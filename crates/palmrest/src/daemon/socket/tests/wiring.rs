//! The socket thread end to end on a private path: Hello first, fan-out, filters, a subscriber's
//! `PointerOver` becoming the tracker's claim (acceptance 10's socket half). Decisions are tested
//! as pure steps in `step`, `packet` and `aggregate`.

use super::support::{connect, me, recv, seat_file, send};
use crate::daemon::socket::{FromSocket, Steering, ToSocket, bind, spawn};
use palmrest_protocol::{
    ActiveTouch, DeviceInfo, Filter, Over, PROTOCOL_VERSION, TapKind, ToDaemon, ToHost, Topic,
};
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn hello_fanout_filters_and_claims() {
    let dir = std::env::temp_dir().join(format!("palmrest-sock-{}", std::process::id()));
    let path = dir.join("events.sock");
    let listener = bind(&path).unwrap();
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o666,
        "open to every account; the server decides whom to serve"
    );
    let (claims_tx, claims) = mpsc::channel();
    // Every program of the test user stands for the shell: these clients are in no unit.
    let handle = spawn(
        listener,
        claims_tx,
        seat_file(&dir, Some(me())),
        Steering::SeatUser,
    )
    .unwrap();

    let a = connect(&path);
    assert!(matches!(
        recv(&a),
        ToHost::Hello {
            version: PROTOCOL_VERSION,
            ..
        }
    ));
    assert_eq!(recv(&a), ToHost::DeviceGone, "no mouse yet");

    let device = DeviceInfo {
        name: "mouse".into(),
        path: "/dev/hidraw9".into(),
        vendor: 0x4c,
        product: 0x269,
    };
    handle.send(ToSocket::Device(device.clone()));
    assert_eq!(
        recv(&a),
        ToHost::Hello {
            version: PROTOCOL_VERSION,
            device: device.clone()
        }
    );

    let b = connect(&path);
    assert_eq!(
        recv(&b),
        ToHost::Hello {
            version: PROTOCOL_VERSION,
            device
        },
        "late subscribers learn the device"
    );
    send(
        &b,
        &ToDaemon::Subscribe {
            filter: Filter::Only(vec![Topic::Tap]),
        },
    );

    send(&a, &ToDaemon::PointerOver { over: Over::Ours });
    assert_eq!(
        claims.recv_timeout(Duration::from_secs(2)),
        Ok(FromSocket::Pointer(Over::Ours))
    );

    std::thread::sleep(Duration::from_millis(50)); // let b's Subscribe land before the fan-out
    let touches = ToHost::Touches {
        active: ActiveTouch::Down,
    };
    let tap = ToHost::Tap {
        kind: TapKind::Two,
        t_ns: 1,
    };
    handle.send(ToSocket::Msg(touches.clone()));
    handle.send(ToSocket::Msg(tap.clone()));
    assert_eq!(recv(&a), touches);
    assert_eq!(recv(&a), tap);
    assert_eq!(recv(&b), tap, "b filtered Touches out");

    drop(a);
    assert_eq!(
        claims.recv_timeout(Duration::from_secs(2)),
        Ok(FromSocket::Pointer(Over::NotOurs)),
        "a leaving releases its claim"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
