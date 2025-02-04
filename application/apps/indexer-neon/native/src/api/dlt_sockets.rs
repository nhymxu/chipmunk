use crate::{api::dlt_indexing::FormatOptions, channels::EventEmitterTask};
use chrono_tz::Tz;
use crossbeam_channel as cc;
use dlt_core::fibex::FibexConfig;
use dlt_core::filtering::DltFilterConfig;
use indexer_base::{chunks::ChunkResults, config::SocketConfig};
use neon::prelude::*;
use std::{path, thread};
use tokio::sync;

#[derive(Debug)]
pub struct SocketThreadConfig {
    pub out_path: path::PathBuf,
    pub tag: String,
}
pub struct SocketDltEventEmitter {
    pub event_receiver: cc::Receiver<ChunkResults>,
    pub shutdown_sender: sync::mpsc::Sender<()>,
    pub task_thread: Option<std::thread::JoinHandle<()>>,
}
impl SocketDltEventEmitter {
    #[allow(clippy::too_many_arguments)]
    pub fn start_indexing_socket_in_thread(
        self: &mut SocketDltEventEmitter,
        session_id: String,
        shutdown_rx: sync::mpsc::Receiver<()>,
        chunk_result_sender: cc::Sender<ChunkResults>,
        thread_conf: SocketThreadConfig,
        socket_conf: SocketConfig,
        filter_conf: Option<DltFilterConfig>,
        fibex: FibexConfig,
        fmt_options: dlt_core::fmt::FormatOptions,
    ) {
        info!("start_indexing_socket_in_thread: {:?}", thread_conf);
        use tokio::runtime::Runtime;
        // Create the runtime
        let rt = Runtime::new().expect("Could not create runtime");
        // Ask:: why self.task_thread has to be stored?
        // Ask:: why async task wrapped into thread?
        // Spawn a thread to continue running after this method has returned.
        self.task_thread = Some(thread::spawn(move || {
            rt.block_on(async {
                let socket_future = dlt::dlt_net::create_index_and_mapping_dlt_from_socket(
                    session_id,
                    socket_conf,
                    thread_conf.tag.as_str(),
                    &thread_conf.out_path,
                    filter_conf,
                    &chunk_result_sender,
                    // &tx,
                    shutdown_rx,
                    Some(fibex),
                    fmt_options,
                );
                match socket_future.await {
                    Ok(_) => {}
                    Err(e) => warn!("error for socket dlt stream: {}", e),
                }
                debug!("Back after DLT indexing finished!");
            });
        }));
        debug!("Thread for dlt socket is running");
    }
}

// interface of the Rust code for js, exposes the `poll` and `shutdown` methods
declare_types! {
    pub class JsDltSocketEventEmitter for SocketDltEventEmitter {
        init(mut cx) {
            trace!("Rust: JsDltSocketEventEmitter");
            let session_id = cx.argument::<JsString>(0)?.value();
            let arg_socket_conf = cx.argument::<JsValue>(1)?;
            let socket_conf: SocketConfig = neon_serde::from_value(&mut cx, arg_socket_conf)?;
            let tag = cx.argument::<JsString>(2)?.value();
            let out_path = path::PathBuf::from(cx.argument::<JsString>(3)?.value().as_str());
            let arg_filter_conf = cx.argument::<JsValue>(4)?;
            let filter_conf: DltFilterConfig = neon_serde::from_value(&mut cx, arg_filter_conf)?;

            let arg_fibex_conf = cx.argument::<JsValue>(5)?;
            let fibex_conf: FibexConfig = neon_serde::from_value(&mut cx, arg_fibex_conf)?;
            let arg_fmt_options = cx.argument::<JsValue>(6)?;
            let fmt_options_in: FormatOptions = neon_serde::from_value(&mut cx, arg_fmt_options)?;
            let mut fmt_options = dlt_core::fmt::FormatOptions { tz: None };
            if let Some(tz_str) = fmt_options_in.tz {
                trace!("will try to use timezone: {}", tz_str);
                match tz_str.parse::<Tz>() {
                    Ok(tz) => {
                        fmt_options.tz = Some(tz);
                    },
                    Err(err) => {
                        warn!("fail to get timezone from: {}; error: {}", tz_str, err);
                    }
                }
            }
            let shutdown_channel = sync::mpsc::channel(1);
            let (tx, rx): (cc::Sender<ChunkResults>, cc::Receiver<ChunkResults>) = cc::unbounded();
            let mut emitter = SocketDltEventEmitter {
                event_receiver: rx,
                shutdown_sender: shutdown_channel.0,
                task_thread: None,
            };

            emitter.start_indexing_socket_in_thread(
                session_id,
                shutdown_channel.1,
                tx,
                SocketThreadConfig {
                    out_path,
                    tag,
                },
                socket_conf,
                Some(filter_conf),
                fibex_conf,
                fmt_options,
            );
            Ok(emitter)
        }

        // will be called by JS to receive data in a loop, but care should be taken to only call it once at a time.
        method poll(mut cx) {
            // The callback to be executed when data is available
            let cb = cx.argument::<JsFunction>(0)?;
            let this = cx.this();

            // Create an asynchronously `EventEmitterTask` to receive data
            let events = cx.borrow(&this, |emitter| emitter.event_receiver.clone());
            let emitter = EventEmitterTask::new(events);

            // Schedule the task on the `libuv` thread pool
            emitter.schedule(cb);
            Ok(JsUndefined::new().upcast())
        }

        // The shutdown method may be called to stop the Rust thread. It
        // will error if the thread has already been destroyed.
        method shutdown(mut cx) {
            use tokio::runtime::Runtime;
            trace!("shutdown called");
            let this = cx.this();
            // Create the runtime
            let rt = Runtime::new().expect("Could not create runtime");

            // Unwrap the shutdown channel and send a shutdown command
            cx.borrow(&this, |emitter| {
                rt.block_on(
                    async {
                        let _ = emitter.shutdown_sender.send(()).await;
                        trace!("sent command Shutdown")
                    }
                );
            });
            Ok(JsUndefined::new().upcast())
        }
    }
}
