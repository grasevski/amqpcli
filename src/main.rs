//! AMQP command line interface.
use amq_protocol_types::FieldTable;
use futures_lite::stream::StreamExt;
use lapin::{
    options::{BasicConsumeOptions, BasicPublishOptions},
    BasicProperties, Channel, Connection, ConnectionProperties,
};
use mimalloc::MiMalloc;
use std::io::{stdin, BufRead};
use structopt::StructOpt;

/// A fast cross platform allocator.
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

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
        #[structopt(short, long, default_value  = "")]
        consumer_tag: String,
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
        match self {
            Self::Consume {
                queue,
                consumer_tag,
            } => {
                let opts = BasicConsumeOptions {
                    no_ack: true,
                    nowait: true,
                    ..BasicConsumeOptions::default()
                };
                let mut consumer = chan
                    .basic_consume(&queue, &consumer_tag, opts, FieldTable::default())
                    .await
                    .unwrap();
                while let Some(delivery) = consumer.next().await {
                    println!("{}", std::str::from_utf8(&delivery.unwrap().data).unwrap());
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
