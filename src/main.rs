//! AMQP command line interface.
use amq_protocol_types::FieldTable;
use core::time::Duration;
use futures_lite::stream::StreamExt;
use lapin::{
    options::{
        BasicAckOptions, BasicConsumeOptions, BasicPublishOptions, BasicQosOptions,
        BasicRejectOptions,
    },
    BasicProperties, Channel, Connection, ConnectionProperties,
};
use mimalloc::MiMalloc;
use std::io::{stdin, BufRead};
use structopt::StructOpt;

/// A fast cross platform allocator.
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// Sets up stdio and runs the application.
#[tokio::main]
async fn main() {
    reset_signal_pipe_handler();
    Opts::from_args().run().await;
}

/// Handle pipe output.
fn reset_signal_pipe_handler() {
    #[cfg(target_family = "unix")]
    {
        use nix::sys::signal;
        unsafe { signal::signal(signal::Signal::SIGPIPE, signal::SigHandler::SigDfl) }.unwrap();
    }
}

/// Command line interface to publish and consume rabbitmq messages.
#[derive(StructOpt)]
struct Opts {
    /// Broker address.
    #[structopt(short, long, default_value = "amqp://localhost:5672/%2f")]
    addr: String,

    /// Command to run against rabbitmq.
    #[structopt(subcommand)]
    cmd: Cmd,
}

impl Opts {
    /// Connects to rabbitmq and runs the desired command.
    async fn run(self) {
        let conn = Connection::connect(&self.addr, ConnectionProperties::default())
            .await
            .unwrap();
        self.cmd.run(conn.create_channel().await.unwrap()).await;
    }
}

/// Commands which can be run against rabbitmq broker.
#[derive(StructOpt)]
enum Cmd {
    /// Reads messages from rabbitmq and writes them line by line to stdout.
    Consume {
        /// The queue from which to read.
        queue: String,

        /// Identifies the connection.
        #[structopt(short, long, default_value = "")]
        consumer_tag: String,

        /// Whether to acknowledge messages containing newlines.
        #[structopt(short, long)]
        newline_error_ack: bool,

        /// Whether to acknowledge messages which cannot be parsed as utf-8.
        #[structopt(short, long)]
        parse_error_ack: bool,
    },

    /// Reads messages line by line from stdin and writes them to rabbitmq.
    Publish {
        /// Destination exchange.
        #[structopt(short, long, default_value = "")]
        exchange: String,

        /// Routing key for all messages.
        #[structopt(short, long, default_value = "")]
        routing_key: String,
    },
}

impl Cmd {
    /// Loops through the messages line by line.
    async fn run(self, chan: Channel) {
        const BATCH_SIZE: u16 = 0x100;
        match self {
            Self::Consume {
                queue,
                consumer_tag,
                newline_error_ack,
                parse_error_ack,
            } => {
                chan.basic_qos(BATCH_SIZE << 1, BasicQosOptions::default())
                    .await
                    .unwrap();
                let mut consumer = chan
                    .basic_consume(
                        &queue,
                        &consumer_tag,
                        BasicConsumeOptions::default(),
                        FieldTable::default(),
                    )
                    .await
                    .unwrap();
                let (mut i, mut acker) = (0, None);
                loop {
                    if let Ok(delivery) =
                        tokio::time::timeout(Duration::new(1, 0), consumer.next()).await
                    {
                        let delivery = delivery.unwrap().unwrap();
                        match std::str::from_utf8(&delivery.data) {
                            Ok(data) => {
                                if data.contains('\n') {
                                    eprintln!("message contains newlines: {}", data);
                                    if newline_error_ack {
                                        acker = Some(delivery.acker);
                                    } else {
                                        delivery
                                            .acker
                                            .reject(BasicRejectOptions::default())
                                            .await
                                            .unwrap();
                                    }
                                } else {
                                    acker = Some(delivery.acker);
                                    println!("{}", data);
                                }
                            }
                            Err(err) => {
                                eprintln!("parse error: {}", err);
                                if parse_error_ack {
                                    acker = Some(delivery.acker);
                                } else {
                                    delivery
                                        .acker
                                        .reject(BasicRejectOptions::default())
                                        .await
                                        .unwrap();
                                }
                            }
                        }
                        i += 1;
                        if i == BATCH_SIZE {
                            acker
                                .take()
                                .unwrap()
                                .ack(BasicAckOptions { multiple: true })
                                .await
                                .unwrap();
                            i = 0;
                        }
                    } else if let Some(acker) = acker.take() {
                        acker.ack(BasicAckOptions { multiple: true }).await.unwrap();
                        i = 0;
                    }
                }
            }
            Self::Publish {
                exchange,
                routing_key,
            } => {
                for payload in stdin().lock().lines() {
                    chan.basic_publish(
                        &exchange,
                        &routing_key,
                        BasicPublishOptions::default(),
                        payload.unwrap().as_bytes(),
                        BasicProperties::default(),
                    )
                    .await
                    .unwrap()
                    .await
                    .unwrap();
                }
            }
        }
    }
}
