use capnp;
use capnp::serialize_packed;
use crypto::identity::PublicKey;
use common::int_convert::usize_to_u32;
use crate::capnp_common::{write_signature, read_signature,
                          write_custom_int128, read_custom_int128,
                          write_custom_u_int128, read_custom_u_int128,
                          write_rand_nonce, read_rand_nonce,
                          write_uid, read_uid,
                          write_invoice_id, read_invoice_id,
                          write_public_key, read_public_key,
                          write_relay_address, read_relay_address,
                          write_index_server_address, read_index_server_address,
                          write_receipt, read_receipt};

use report_capnp;
use crate::serialize::SerializeError;
use crate::report::messages::MoveTokenHashedReport;


fn ser_move_token_hashed(move_token_hashed_report: &MoveTokenHashedReport,
                    move_token_hashed_builder: &mut app_server_capnp::reset_friend_channel::Builder) {

    write_hash(&move_token_hashed_report.prefix_hash,
        &mut move_token_hashed_builder.reborrow().init_prefix_hash());

    write_public_key(&move_token_hashed_report.local_public_key,
        &mut move_token_hashed_builder.reborrow().init_local_public_key());

    write_public_key(&move_token_hashed_report.remote_public_key,
        &mut move_token_hashed_builder.reborrow().init_remote_public_key());

    move_token_hashed_builder.reborrow().set_inconsistency_counter(move_token_hashed_report.inconsistency_counter);

    write_custom_u_int128(&move_token_hashed_report.move_token_counter,
        &mut move_token_hashed_builder.reborrow().init_move_token_counter());

    write_custom_int128(&move_token_hashed_report.balance,
        &mut move_token_hashed_builder.reborrow().init_balance());

    write_custom_u_int128(&move_token_hashed_report.local_pending_debt,
        &mut move_token_hashed_builder.reborrow().init_local_pending_debt());

    write_custom_u_int128(&move_token_hashed_report.remote_pending_debt,
        &mut move_token_hashed_builder.reborrow().init_remote_pending_debt());

    write_rand_nonce(&move_token_hashed_report.rand_nonce,
        &mut move_token_hashed_builder.reborrow().init_rand_nonce());

    write_signature(&move_token_hashed_report.new_token,
        &mut move_token_hashed_builder.reborrow().init_new_token());
}

fn deser_move_token_hahsed(move_token_hashed_reader: &app_server_capnp::reset_friend_channel::Reader)
    -> Result<MoveTokenHashedReport, SerializeError> {

    Ok(MoveTokenHashedReport {
        prefix_hash: read_hash(&move_token_hashed_reader.get_prefix_hash()?)?,
        local_public_key: read_public_key(&move_token_hashed_reader.get_local_public_key()?)?,
        remote_public_key: read_public_key(&move_token_hashed_reader.get_remote_public_key()?)?,
        inconsistency_counter: move_token_hashed_reader.get_inconsistency_counter()?,
        move_token_counter: read_custom_u_int128(&move_token_hashed_reader.get_move_token_counter()?)?,
        balance: read_custom_int128(&move_token_hashed_reader.get_balance()?)?,
        local_pending_debt: read_custom_u_int128(&move_token_hashed_reader.get_local_pending_debt()?)?,
        remote_pending_debt: read_custom_u_int128(&move_token_hashed_reader.get_remote_pending_debt()?)?,
        rand_nonce: read_rand_nonce(&move_token_hashed_reader.get_rand_nonce()?)?,
        new_token: read_signature(&move_token_hashed_reader.get_new_token()?)?,
    })
}