// uses
use super::reporter;
use super::supervisor;
use crate::worker::StreamStatus;
use tokio::io::WriteHalf;
use tokio::net::TcpStream;
use tokio::prelude::*;
use tokio::sync::mpsc;

// types
pub type Sender = mpsc::UnboundedSender<Event>;
pub type Receiver = mpsc::UnboundedReceiver<Event>;
// Payload type is vector<unsigned-integer-8bit>.
pub type Payload = Vec<u8>;

#[derive(Debug)]
pub enum Event {
    Payload {
        stream: reporter::Stream,
        payload: Payload,
        reporter_id: u8,
    },
}

// args struct, each actor must have public Arguments struct,
// to pass options when starting the actor.
pub struct SenderBuilder {
    // sender's tx only to pass it to reporters (if recoonect == true).
    tx: Option<Sender>,
    // sender's rx to recv events
    rx: Option<Receiver>,
    stage_tx: Option<supervisor::Sender>,
    socket_tx: Option<WriteHalf<TcpStream>>,
    reporters: Option<supervisor::Reporters>,
    session_id: Option<usize>,
    reconnect: bool,
}

impl SenderBuilder {
    pub fn new() -> Self {
        SenderBuilder {
            tx: None,
            rx: None,
            stage_tx: None,
            socket_tx: None,
            reporters: None,
            session_id: None,
            reconnect: false,
        }
    }

    set_builder_option_field!(tx, Sender);
    set_builder_option_field!(rx, Receiver);
    set_builder_option_field!(stage_tx, supervisor::Sender);
    set_builder_option_field!(socket_tx, WriteHalf<TcpStream>);
    set_builder_option_field!(reporters, supervisor::Reporters);
    set_builder_option_field!(session_id, usize);
    set_builder_field!(reconnect, bool);

    pub fn build(self) -> SenderState {
        let state = SenderState {
            // stage_tx: self.stage_tx.unwrap(),
            reporters: self.reporters.unwrap(),
            session_id: self.session_id.unwrap(),
            socket: self.socket_tx.unwrap(),
            tx: self.tx.unwrap(),
            rx: self.rx.unwrap(),
        };

        if self.reconnect {
            for (_, reporter_tx) in &state.reporters {
                reporter_tx
                    .send(reporter::Event::Session(reporter::Session::New(
                        state.session_id,
                        state.tx.clone(),
                    )))
                    .unwrap();
            }
        }

        state
    }
}

// sender's state struct.
pub struct SenderState {
    reporters: supervisor::Reporters,
    session_id: usize,
    socket: WriteHalf<TcpStream>,
    tx: Sender,
    rx: Receiver,
}

impl SenderState {
    pub async fn run(mut self) {
        // loop to process event by event.
        while let Some(Event::Payload {
            stream,
            payload,
            reporter_id,
        }) = self.rx.recv().await
        {
            // write the payload to the socket, make sure the result is valid
            match self.socket.write_all(&payload).await {
                Ok(()) => {
                    // send to reporter send_status::Ok(stream_id)
                    self.reporters
                        .get(&reporter_id)
                        .unwrap()
                        .send(reporter::Event::StreamStatus(StreamStatus::Ok(stream)))
                        .unwrap();
                }
                Err(_) => {
                    // send to reporter send_status::Err(stream_id)
                    self.reporters
                        .get(&reporter_id)
                        .unwrap()
                        .send(reporter::Event::StreamStatus(StreamStatus::Err(stream)))
                        .unwrap();
                    // close channel to prevent any further Payloads to be sent from reporters
                    self.rx.close();
                }
            }
        } // if sender reached this line, then either write_all returned IO Err(err) or reporter(s) droped sender_tx(s)

        // probably not needed
        self.socket.shutdown().await.unwrap();

        // send checkpoint to all reporters because the socket is mostly closed
        for (_, reporter_tx) in &self.reporters {
            reporter_tx
                .send(reporter::Event::Session(reporter::Session::CheckPoint(
                    self.session_id,
                )))
                .unwrap();
        }
    }
}