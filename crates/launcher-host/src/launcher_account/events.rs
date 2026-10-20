//! The existing account event runtime continuously polls the selected service owners.
use super::*;

/// Drives retained control, presence and product futures on the existing event executor.
pub(super) fn poll(
    shared: &Mutex<Snapshot>,
    requests: &Receiver<()>,
    lifetime: &TitlePresenceLifetime,
    runtime: &Handle,
) {
    let guard = lifetime.worker_guard();
    runtime.block_on(run(shared, requests, lifetime));
    drop(guard);
}

/// Drives the original three retained futures; the existing poll caller owns their worker guard.
async fn run(shared: &Mutex<Snapshot>, requests: &Receiver<()>, lifetime: &TitlePresenceLifetime) {
    let control = async {
        control(shared, requests, lifetime).await;
        lifetime.retire();
    };
    let publisher = async {
        #[cfg(all(feature = "native-networking", not(target_arch = "wasm32")))]
        native_presence::run(shared, lifetime).await;
        #[cfg(not(all(feature = "native-networking", not(target_arch = "wasm32"))))]
        {
            lifetime.retiring().await;
            lifetime.complete(true);
        }
    };
    let product = async {
        #[cfg(all(feature = "native-networking", not(target_arch = "wasm32")))]
        {
            let (api, account) = {
                let snapshot = shared.lock().unwrap_or_else(|p| p.into_inner());
                (
                    snapshot.native_product_api.clone(),
                    snapshot.native_presence_account.clone(),
                )
            };
            if let Some(api) = api {
                tokio::select! { biased;
                    _=lifetime.retiring()=>{},
                    _=api.run(account)=>{},
                }
                api.finish();
            }
        }
    };
    tokio::join!(control, publisher, product);
}

/// Retains each control round while the publisher makes independent progress on the same executor.
async fn control(
    shared: &Mutex<Snapshot>,
    requests: &Receiver<()>,
    lifetime: &TitlePresenceLifetime,
) {
    let mut seen_release = None;
    let mut ping_due = Instant::now();
    let mut pinged: Vec<String> = Vec::new();
    loop {
        let joining = shared
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .joining;
        let interval = if joining {
            JOIN_EVENT_INTERVAL
        } else {
            EVENT_INTERVAL
        };
        if lifetime.retirement_requested() {
            return;
        }
        tokio::select! {
            _ = lifetime.notified() => {},
            _ = tokio::time::sleep(interval) => {},
        }
        match requests.try_recv() {
            Ok(()) => {
                lifetime.retire();
                let api = shared
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .native_product_api
                    .clone();
                if let Some(api) = api {
                    api.stop();
                    api.finished().await;
                }
                lifetime.finished().await;
                return;
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                lifetime.retire();
                return;
            }
            Err(crossbeam_channel::TryRecvError::Empty) => {}
        }
        if lifetime.retirement_requested() {
            return;
        }
        let (cinnabar, generation) = {
            let snapshot = shared.lock().unwrap_or_else(|p| p.into_inner());
            (snapshot.cinnabar_api.clone(), snapshot.auth_generation)
        };
        #[cfg(all(feature = "native-networking", not(target_arch = "wasm32")))]
        {
            let announced = {
                let snapshot = shared.lock().unwrap_or_else(|p| p.into_inner());
                if snapshot.same_generation(generation)
                    && let Some(api) = &snapshot.native_product_api
                {
                    if let Some(config) = &cinnabar {
                        api.configure(config.clone());
                    }
                    let release = api.take_release();
                    if new_release(&mut seen_release, release.as_ref()).is_some() {
                        release
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            if let Some(release) = announced {
                crate::lifecycle::update::release_published(&release);
            }
        }
        let targets = shared
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .ping_targets
            .clone();
        // The feeds worker publishes first, so rows it adds are new targets here
        // and are pinged on the next tick instead of a round later.
        if targets != pinged {
            ping_due = Instant::now();
        }
        if !targets.is_empty() && Instant::now() >= ping_due {
            ping_due = Instant::now() + PING_INTERVAL;
            pinged = targets.clone();
            let pongs = tokio::select! { biased;
                _ = lifetime.retiring() => return,
                pongs = server_ping::round(&targets) => pongs,
            };
            let pings = round_results(&targets, pongs);
            publish(shared, |snapshot| snapshot.pings = Some(pings));
        }
    }
}

#[cfg(test)]
mod tests;
