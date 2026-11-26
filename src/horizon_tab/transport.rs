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

/// A RabbitMQ link: one fanout exchange, each instance with its own exclusive queue bound to it. The thread
/// owns a current-thread tokio runtime and reconnects every five seconds after an error.
pub struct Rabbit {
    out: tokio::sync::mpsc::UnboundedSender<String>,
    inbox: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
}

impl Rabbit {
    pub fn start(url: &str, exchange: &str, channel: &str) -> Result<Rabbit, String> {
        if !url.starts_with("amqp://") && !url.starts_with("amqps://") {
            return Err(format!("rabbitmq: `{url}` is not an AMQP URI"));
        }
        let (out, mut orx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let inbox = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (url, exchange, channel) = (url.to_owned(), exchange.to_owned(), channel.to_owned());
        let (inb, flag) = (inbox.clone(), stop.clone());
        std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
            rt.block_on(async move {
                use lapin::options::{BasicGetOptions, BasicPublishOptions, ExchangeDeclareOptions, QueueBindOptions, QueueDeclareOptions};
                use lapin::types::FieldTable;
                while !flag.load(Ordering::Relaxed) {
                    let attempt = async {
                        let conn = lapin::Connection::connect(&url, lapin::ConnectionProperties::default()).await?;
                        let ch = conn.create_channel().await?;
                        ch.exchange_declare(exchange.as_str().into(), lapin::ExchangeKind::Fanout, ExchangeDeclareOptions { durable: false, ..Default::default() }, FieldTable::default()).await?;
                        let q = ch.queue_declare("".into(), QueueDeclareOptions { durable: false, exclusive: true, auto_delete: true, ..Default::default() }, FieldTable::default()).await?;
                        ch.queue_bind(q.name().clone(), exchange.as_str().into(), channel.as_str().into(), QueueBindOptions::default(), FieldTable::default()).await?;
                        while !flag.load(Ordering::Relaxed) {
                            while let Ok(m) = orx.try_recv() {
                                ch.basic_publish(exchange.as_str().into(), channel.as_str().into(), BasicPublishOptions::default(), m.as_bytes(), lapin::BasicProperties::default()).await?;
                            }
                            while let Some(m) = ch.basic_get(q.name().clone(), BasicGetOptions { no_ack: true }).await? {
                                inb.lock().unwrap_or_else(|e| e.into_inner()).push(String::from_utf8_lossy(&m.data).into_owned());
                            }
                            tokio::time::sleep(Duration::from_millis(200)).await;
                        }
                        Ok::<(), lapin::Error>(())
                    };
                    if attempt.await.is_err() {
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            });
        });
        Ok(Rabbit { out, inbox, stop })
    }

    pub fn send(&self, message: String) {
        let _ = self.out.send(message);
    }

    pub fn drain(&self) -> Vec<String> {
        std::mem::take(&mut *self.inbox.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// The running broker link, whichever kind.
pub enum Link {
    Redis(Redis),
    Rabbit(Rabbit),
}

impl Link {
    pub fn send(&self, message: String) {
        match self {
            Link::Redis(l) => l.send(message),
            Link::Rabbit(l) => l.send(message),
        }
    }
    pub fn drain(&self) -> Vec<String> {
        match self {
            Link::Redis(l) => l.drain(),
            Link::Rabbit(l) => l.drain(),
        }
    }
    pub fn stop(&self) {
        match self {
            Link::Redis(l) => l.stop(),
            Link::Rabbit(l) => l.stop(),
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
    fn a_rabbitmq_uri_is_checked_and_an_unreachable_broker_stays_quiet() {
        assert!(Rabbit::start("localhost", "plugin", "TAB_4_net").is_err());
        let r = Rabbit::start("amqp://guest:guest@127.0.0.1:1/%2F", "plugin", "TAB_4_net").unwrap();
        r.send("x".into());
        assert!(r.drain().is_empty());
        r.stop();
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
