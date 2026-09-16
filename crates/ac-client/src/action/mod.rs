//! What a front end asks the character to do, in one vocabulary: the chat box,
//! a key, a Rhai call, the bus, the CLI and a panel button all build an
//! [`Action`](self) and hand it to `Client::act`. `act` carries external intent
//! only: autoplay decides for itself and calls the same methods directly, and
//! so does every read.

pub mod resolve;
