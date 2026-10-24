//! A manual probe: runs one real ceremony against the platform and prints the
//! outcome. On macOS it opens a window so the system has something to present
//! over. Usage: `cargo run -p aimer_passkey --example probe -- [create|get] [rp-id]`.

#[cfg(target_os = "macos")]
fn main() {
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll, Waker};

    use aimer_passkey::{CreateRequest, GetRequest, Passkeys, PlatformPasskeys, RelyingParty, User};
    use objc2::MainThreadMarker;
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSWindow,
        NSWindowStyleMask,
    };
    use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSPoint, NSRect, NSRunLoop, NSSize};

    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| "get".into());
    let rp_id = args.next().unwrap_or_else(|| "example.com".into());

    let mtm = MainThreadMarker::new().expect("main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            mtm.alloc(),
            NSRect::new(NSPoint::new(200.0, 200.0), NSSize::new(480.0, 320.0)),
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    window.makeKeyAndOrderFront(None);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);

    let passkeys = PlatformPasskeys::new();
    let mut available = passkeys.is_available();
    let mut cx = Context::from_waker(Waker::noop());
    println!("available: {:?}", Pin::new(&mut available).poll(&mut cx));

    let challenge = vec![7u8; 32];
    let future: Pin<Box<dyn Future<Output = String>>> = if mode == "create" {
        let request = CreateRequest::new(
            RelyingParty::new(&rp_id, "Probe"),
            User::new(vec![1u8; 16], "probe@example.com"),
            challenge,
        );
        let ceremony = passkeys.create(request);
        Box::pin(async move { format!("{:?}", ceremony.await) })
    } else {
        let ceremony = passkeys.get(GetRequest::new(&rp_id, challenge));
        Box::pin(async move { format!("{:?}", ceremony.await) })
    };
    let mut future = future;
    let started = std::time::Instant::now();
    loop {
        if let Poll::Ready(outcome) = future.as_mut().poll(&mut cx) {
            println!("{mode} -> {outcome} after {:?}", started.elapsed());
            break;
        }
        if started.elapsed().as_secs() > 20 {
            println!("still pending after 20 s (a system sheet is probably up); dropping to cancel");
            break;
        }
        unsafe {
            NSRunLoop::currentRunLoop()
                .runMode_beforeDate(NSDefaultRunLoopMode, &NSDate::dateWithTimeIntervalSinceNow(0.05));
        }
    }
    drop(future);
    drop(window);
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("the probe only drives a window on macOS");
}
