//! Form input and transport systems; retained state lives in client-ui.
use client_ui::ui_runtime::forms::{
    FormTransportError, LocalFormAction, engine_input, flush_form_response,
};
mod interaction;
mod network;

pub(crate) use interaction::{drive_server_form_input, track_server_form_input_mode};
pub(crate) use network::flush_server_form_network;
