//! The transports of `proxy-support` (TAB's `ProxySupport`): the messages `proxy.rs` encodes (Base64
//! text) travel as plugin messages (`PLUGIN`, through a player's connection to the proxy) or through
//! a Redis pub/sub channel (`REDIS`). The lib glue owns the plugin channel calls; this file has the
//! channel naming and the Redis link, free of game types.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The plugin channel of a `proxy-support` channel name (`TAB_4_net` -> `tab:tab_4_net`): an identifier
/// allows lower case letters, digits and `_-./` only.
pub fn plugin_channel(name: &str) -> String {
    let path: String = name
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/') { c } else { '_' })
        .collect();
    format!("tab:{path}")
}

/// A Redis pub/sub link on one channel: a listener thread fills the inbox, a publisher thread drains the
/// send queue. Both reconnect after an error until [`Redis::stop`].
pub struct Redis {
    tx: Sender<String>,
    inbox: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
}

impl Redis {
    /// Opens the link (the client only parses the URL; connecting happens in the threads).
    pub fn start(url: &str, channel: &str) -> Result<Redis, String> {
        let client = redis::Client::open(url).map_err(|e| format!("redis: {e}"))?;
        let inbox = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx): (Sender<String>, Receiver<String>) = mpsc::channel();
        {
            let (client, inbox, stop, channel) = (client.clone(), inbox.clone(), stop.clone(), channel.to_owned());
            std::thread::spawn(move || listen(&client, &channel, &inbox, &stop));
        }
        {
            let (stop, channel) = (stop.clone(), channel.to_owned());
            std::thread::spawn(move || publish(&client, &channel, &rx, &stop));
        }
        Ok(Redis { tx, inbox, stop })
    }

    /// Queues one message for the channel.
    pub fn send(&self, message: String) {
        let _ = self.tx.send(message);
    }

    /// The messages received since the last call.
    pub fn drain(&self) -> Vec<String> {
        std::mem::take(&mut *self.inbox.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Ends both threads (within a second).
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn listen(client: &redis::Client, channel: &str, inbox: &Mutex<Vec<String>>, stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        let Ok(mut con) = client.get_connection() else {
            std::thread::sleep(Duration::from_secs(2));
            continue;
        };
        let mut pubsub = con.as_pubsub();
        let _ = pubsub.set_read_timeout(Some(Duration::from_millis(500)));
        if pubsub.subscribe(channel).is_err() {
            std::thread::sleep(Duration::from_secs(2));
            continue;
        }
        while !stop.load(Ordering::Relaxed) {
            match pubsub.get_message() {
                Ok(msg) => {
                    let text = String::from_utf8_lossy(msg.get_payload_bytes()).into_owned();
                    inbox.lock().unwrap_or_else(|e| e.into_inner()).push(text);
                }
                Err(e) if e.is_timeout() => {}
                Err(_) => break,
            }
        }
    }
}

fn publish(client: &redis::Client, channel: &str, rx: &Receiver<String>, stop: &AtomicBool) {
    let mut con = None;
    while !stop.load(Ordering::Relaxed) {
        let Ok(message) = rx.recv_timeout(Duration::from_millis(500)) else { continue };
        for _ in 0..2 {
            if con.is_none() {
                con = client.get_connection().ok();
            }
            let Some(c) = con.as_mut() else { break };
            if redis::cmd("PUBLISH").arg(channel).arg(&message).query::<i64>(c).is_ok() {
                break;
            }
            con = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_channel_is_a_valid_identifier() {
        assert_eq!(plugin_channel("TAB_4_net"), "tab:tab_4_net");
        assert_eq!(plugin_channel("TAB_4_my suffix!"), "tab:tab_4_my_suffix_");
    }

    #[test]
    fn a_bad_redis_url_is_refused_and_a_good_one_starts_quietly() {
        assert!(Redis::start("not a url", "TAB_4_net").is_err());
        let r = Redis::start("redis://127.0.0.1:1/0", "TAB_4_net").unwrap();
        r.send("x".into());
        assert!(r.drain().is_empty());
        r.stop();
    }
}
