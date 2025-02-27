pub mod fetch;
pub mod lists;
pub mod query;
pub mod send;
pub mod store;

use crate::{
    context::Context,
    entry::CanPublish,
    network::{
        direct_message::DirectMessage,
        entry_aspect::EntryAspect,
        handler::{
            fetch::*,
            lists::{handle_get_authoring_list, handle_get_gossip_list},
            query::*,
            send::*,
            store::*,
        },
    },
    nucleus,
    workflows::get_entry_result::get_entry_with_meta_workflow,
};
use boolinator::*;
use holochain_core_types::{eav::Attribute, entry::Entry, error::HolochainError, time::Timeout};
use holochain_json_api::json::JsonString;
use holochain_net::connection::net_connection::NetHandler;
use holochain_persistence_api::cas::content::Address;
use lib3h_protocol::{
    data_types::{DirectMessageData, StoreEntryAspectData},
    protocol_server::Lib3hServerProtocol,
};
use std::{convert::TryFrom, sync::Arc};

// FIXME: Temporary hack to ignore messages incorrectly sent to us by the networking
// module that aren't really meant for us
fn is_my_dna(my_dna_address: &String, dna_address: &String) -> bool {
    my_dna_address == dna_address
}

// FIXME: Temporary hack to ignore messages incorrectly sent to us by the networking
// module that aren't really meant for us
fn is_my_id(context: &Arc<Context>, agent_id: &str) -> bool {
    if agent_id != "" && context.agent_id.pub_sign_key != agent_id {
        log_debug!(context, "net/handle: ignoring, same id");
        return false;
    }
    true
}

// Since StoreEntryAspectData lives in the net crate and EntryAspect is specific
// to core we can't implement fmt::Debug so that it spans over both, StoreEntryAspectData
// and the type that is represented as opaque byte vector.
// For debug logs we do want to see the whole store request including the EntryAspect.
// This function enables pretty debug logs by deserializing the EntryAspect explicitly
// and combining it with the top-level fields in a formatted and indented output.
fn format_store_data(data: &StoreEntryAspectData) -> String {
    let aspect_json =
        JsonString::from_json(&String::from_utf8(data.entry_aspect.aspect.clone()).unwrap());
    let aspect = EntryAspect::try_from(aspect_json).unwrap();
    format!(
        r#"
StoreEntryAspectData {{
    request_id: "{req_id}",
    dna_address: "{dna_adr}",
    provider_agent_id: "{provider_agent_id}",
    entry_address: "{entry_address}",
    entry_aspect: {{
        aspect_address: "{aspect_address}",
        type_hint: "{type_hint}",
        aspect: "{aspect:?}"
    }}
}}"#,
        req_id = data.request_id,
        dna_adr = data.space_address,
        provider_agent_id = data.provider_agent_id,
        entry_address = data.entry_address,
        aspect_address = data.entry_aspect.aspect_address,
        type_hint = data.entry_aspect.type_hint,
        aspect = aspect
    )
}

// See comment on fn format_store_data() - same reason for this function.
fn format_message_data(data: &DirectMessageData) -> String {
    let message_json = JsonString::from_json(&String::from_utf8(data.content.clone()).unwrap());
    let message = DirectMessage::try_from(message_json).unwrap();
    format!(
        r#"
MessageData {{
    request_id: "{req_id}",
    dna_address: "{dna_adr}",
    to_agent_id: "{to}",
    from_agent_id: "{from}",
    content: {content:?},
}}"#,
        req_id = data.request_id,
        dna_adr = data.space_address,
        to = data.to_agent_id,
        from = data.from_agent_id,
        content = message,
    )
}

/// Creates the network handler.
/// The returned closure is called by the network thread for every network event that core
/// has to handle.
pub fn create_handler(c: &Arc<Context>, my_dna_address: String) -> NetHandler {
    let context = c.clone();
    NetHandler::new(Box::new(move |message| {
        let message = message.unwrap();
        // log_trace!(context, "net/handle:({}): {:?}",
        //   context.agent_id.nick, message
        // );

        let maybe_json_msg = Lib3hServerProtocol::try_from(message);
        if let Err(_) = maybe_json_msg {
            return Ok(());
        }
        match maybe_json_msg.unwrap() {
            Lib3hServerProtocol::FailureResult(failure_data) => {
                if !is_my_dna(&my_dna_address, &failure_data.space_address.to_string()) {
                    return Ok(());
                }
                log_warn!(context, "net/handle: FailureResult: {:?}", failure_data);
                // TODO: Handle the reception of a FailureResult
            }
            Lib3hServerProtocol::HandleStoreEntryAspect(dht_entry_data) => {
                if !is_my_dna(&my_dna_address, &dht_entry_data.space_address.to_string()) {
                    return Ok(());
                }
                log_debug!(context,
                    "net/handle: HandleStoreEntryAspect: {}",
                    format_store_data(&dht_entry_data)
                );
                handle_store(dht_entry_data, context.clone())
            }
            Lib3hServerProtocol::HandleFetchEntry(fetch_entry_data) => {
                if !is_my_dna(&my_dna_address, &fetch_entry_data.space_address.to_string()) {
                    return Ok(());
                }
                log_debug!(context,
                    "net/handle: HandleFetchEntry: {:?}",
                    fetch_entry_data
                );
                handle_fetch_entry(fetch_entry_data, context.clone())
            }
            Lib3hServerProtocol::FetchEntryResult(fetch_result_data) => {
                if !is_my_dna(
                    &my_dna_address,
                    &fetch_result_data.space_address.to_string(),
                ) {
                    return Ok(());
                }

                log_error!(context,
                    "net/handle: unexpected HandleFetchEntryResult: {:?}",
                    fetch_result_data
                );
            }
            Lib3hServerProtocol::HandleQueryEntry(query_entry_data) => {
                if !is_my_dna(&my_dna_address, &query_entry_data.space_address.to_string()) {
                    return Ok(());
                }
                log_debug!(context,
                    "net/handle: HandleQueryEntry: {:?}",
                    query_entry_data
                );
                handle_query_entry_data(query_entry_data, context.clone())
            }
            Lib3hServerProtocol::QueryEntryResult(query_entry_result_data) => {
                if !is_my_dna(
                    &my_dna_address,
                    &query_entry_result_data.space_address.to_string(),
                ) {
                    return Ok(());
                }
                // ignore if I'm not the requester
                if !is_my_id(
                    &context,
                    &query_entry_result_data.requester_agent_id.to_string(),
                ) {
                    return Ok(());
                }
                log_debug!(context,
                    "net/handle: HandleQueryEntryResult: {:?}",
                    query_entry_result_data
                );
                handle_query_entry_result(query_entry_result_data, context.clone())
            }
            Lib3hServerProtocol::HandleSendDirectMessage(message_data) => {
                if !is_my_dna(&my_dna_address, &message_data.space_address.to_string()) {
                    return Ok(());
                }
                // ignore if it's not addressed to me
                if !is_my_id(&context, &message_data.to_agent_id.to_string()) {
                    return Ok(());
                }
                log_debug!(context,
                    "net/handle: HandleSendMessage: {}",
                    format_message_data(&message_data)
                );
                handle_send_message(message_data, context.clone())
            }
            Lib3hServerProtocol::SendDirectMessageResult(message_data) => {
                if !is_my_dna(&my_dna_address, &message_data.space_address.to_string()) {
                    return Ok(());
                }
                // ignore if it's not addressed to me
                if !is_my_id(&context, &message_data.to_agent_id.to_string()) {
                    return Ok(());
                }
                log_debug!(context,
                    "net/handle: SendMessageResult: {}",
                    format_message_data(&message_data)
                );
                handle_send_message_result(message_data, context.clone())
            }
            Lib3hServerProtocol::Connected(peer_data) => {
                log_debug!(context, "net/handle: Connected: {:?}", peer_data);
                return Ok(());
            }
            Lib3hServerProtocol::HandleGetAuthoringEntryList(get_list_data) => {
                if !is_my_dna(&my_dna_address, &get_list_data.space_address.to_string()) {
                    return Ok(());
                }
                // ignore if it's not addressed to me
                if !is_my_id(&context, &get_list_data.provider_agent_id.to_string()) {
                    return Ok(());
                }

                handle_get_authoring_list(get_list_data, context.clone());
            }
            Lib3hServerProtocol::HandleGetGossipingEntryList(get_list_data) => {
                if !is_my_dna(&my_dna_address, &get_list_data.space_address.to_string()) {
                    return Ok(());
                }
                // ignore if it's not addressed to me
                if !is_my_id(&context, &get_list_data.provider_agent_id.to_string()) {
                    return Ok(());
                }

                handle_get_gossip_list(get_list_data, context.clone());
            }
            _ => {}
        }
        Ok(())
    }))
}

fn get_content_aspect(
    entry_address: &Address,
    context: Arc<Context>,
) -> Result<EntryAspect, HolochainError> {
    let entry_with_meta =
        nucleus::actions::get_entry::get_entry_with_meta(&context, entry_address.clone())?
            .ok_or(HolochainError::EntryNotFoundLocally)?;

    let _ = entry_with_meta
        .entry
        .entry_type()
        .can_publish(&context)
        .ok_or(HolochainError::EntryIsPrivate)?;

    let headers = context
        .state()
        .expect("Could not get state for handle_fetch_entry")
        .get_headers(entry_address.clone())
        .map_err(|error| {
            let err_message = format!(
                "net/fetch/get_content_aspect: Error trying to get headers {:?}",
                error
            );
            log_error!(context, "{}", err_message.clone());
            HolochainError::ErrorGeneric(err_message)
        })?;

    // TODO: this is just taking the first header..
    // We should actually transform all headers into EntryAspect::Headers and just the first one
    // into an EntryAspect content (What about ordering? Using the headers timestamp?)
    Ok(EntryAspect::Content(
        entry_with_meta.entry,
        headers[0].clone(),
    ))
}

fn get_meta_aspects(
    entry_address: &Address,
    context: Arc<Context>,
) -> Result<Vec<EntryAspect>, HolochainError> {
    let eavis = context
        .state()
        .expect("Could not get state for handle_fetch_entry")
        .dht()
        .get_all_metas(entry_address)?;

    let (aspects, errors): (Vec<_>, Vec<_>) = eavis
        .iter()
        .filter(|eavi| match eavi.attribute() {
            Attribute::LinkTag(_, _) => true,
            Attribute::RemovedLink(_, _) => true,
            Attribute::CrudLink => true,
            _ => false,
        })
        .map(|eavi| {
            let value_entry = context
                .block_on(get_entry_with_meta_workflow(
                    &context,
                    &eavi.value(),
                    &Timeout::default(),
                ))?
                .ok_or(HolochainError::from(
                    "Entry linked in EAV not found! This should never happen.",
                ))?;
            let header = value_entry.headers[0].to_owned();

            match eavi.attribute() {
                Attribute::LinkTag(_, _) => {
                    let link_data = unwrap_to!(value_entry.entry_with_meta.entry => Entry::LinkAdd);
                    Ok(EntryAspect::LinkAdd(link_data.clone(), header))
                }
                Attribute::RemovedLink(_, _) => {
                    let (link_data, removed_link_entries) =
                        unwrap_to!(value_entry.entry_with_meta.entry => Entry::LinkRemove);
                    Ok(EntryAspect::LinkRemove(
                        (link_data.clone(), removed_link_entries.clone()),
                        header,
                    ))
                }
                Attribute::CrudLink => Ok(EntryAspect::Update(
                    value_entry.entry_with_meta.entry,
                    header,
                )),
                _ => unreachable!(),
            }
        })
        .partition(Result::is_ok);

    if errors.len() > 0 {
        Err(errors[0].to_owned().err().unwrap())
    } else {
        Ok(aspects.into_iter().map(Result::unwrap).collect())
    }
}
