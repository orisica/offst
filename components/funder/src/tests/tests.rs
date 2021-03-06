use futures::executor::ThreadPool;
use futures::task::Spawn;

use crypto::identity::PublicKey;
use crypto::invoice_id::{InvoiceId, INVOICE_ID_LEN};
use crypto::uid::{Uid, UID_LEN};

use proto::funder::messages::{
    FriendStatus, FriendsRoute, FunderControl, FunderIncomingControl, ReceiptAck, RequestsStatus,
    ResetFriendChannel, ResponseSendFundsResult, UserRequestSendFunds,
};
use proto::report::messages::{ChannelStatusReport, FunderReport};

use super::utils::{create_node_controls, dummy_named_relay_address, dummy_relay_address};

async fn task_funder_basic(spawner: impl Spawn + Clone + Send + 'static) {
    let num_nodes = 2;
    let mut node_controls = await!(create_node_controls(num_nodes, spawner));

    let public_keys = node_controls
        .iter()
        .map(|nc| nc.public_key.clone())
        .collect::<Vec<PublicKey>>();

    let relays0 = vec![dummy_relay_address(0)];
    let relays1 = vec![dummy_relay_address(1)];
    await!(node_controls[0].add_friend(&public_keys[1], relays1, "node1", 8));
    await!(node_controls[1].add_friend(&public_keys[0], relays0, "node0", -8));
    assert_eq!(node_controls[0].report.friends.len(), 1);
    assert_eq!(node_controls[1].report.friends.len(), 1);

    await!(node_controls[0].set_friend_status(&public_keys[1], FriendStatus::Enabled));
    await!(node_controls[1].set_friend_status(&public_keys[0], FriendStatus::Enabled));

    // Set remote max debt for both sides:
    await!(node_controls[0].set_remote_max_debt(&public_keys[1], 200));
    await!(node_controls[1].set_remote_max_debt(&public_keys[0], 100));

    // Open requests:
    await!(node_controls[0].set_requests_status(&public_keys[1], RequestsStatus::Open));
    await!(node_controls[1].set_requests_status(&public_keys[0], RequestsStatus::Open));

    // Wait for liveness:
    await!(node_controls[0].wait_until_ready(&public_keys[1]));
    await!(node_controls[1].wait_until_ready(&public_keys[0]));

    // Send credits 0 --> 1
    let user_request_send_funds = UserRequestSendFunds {
        request_id: Uid::from(&[3; UID_LEN]),
        route: FriendsRoute {
            public_keys: vec![
                node_controls[0].public_key.clone(),
                node_controls[1].public_key.clone(),
            ],
        },
        invoice_id: InvoiceId::from(&[1; INVOICE_ID_LEN]),
        dest_payment: 5,
    };
    let incoming_control_message = FunderIncomingControl::new(
        Uid::from(&[40; UID_LEN]),
        FunderControl::RequestSendFunds(user_request_send_funds),
    );
    await!(node_controls[0].send(incoming_control_message)).unwrap();
    let response_received = await!(node_controls[0].recv_until_response()).unwrap();

    assert_eq!(response_received.request_id, Uid::from(&[3; UID_LEN]));
    let receipt = match response_received.result {
        ResponseSendFundsResult::Failure(_) => unreachable!(),
        ResponseSendFundsResult::Success(send_funds_receipt) => send_funds_receipt,
    };

    let receipt_ack = ReceiptAck {
        request_id: Uid::from(&[3; UID_LEN]),
        receipt_signature: receipt.signature.clone(),
    };
    let incoming_control_message = FunderIncomingControl::new(
        Uid::from(&[41; UID_LEN]),
        FunderControl::ReceiptAck(receipt_ack),
    );
    await!(node_controls[0].send(incoming_control_message)).unwrap();

    let pred = |report: &FunderReport<_>| report.num_ready_receipts == 0;
    await!(node_controls[0].recv_until(pred));

    // Verify expected balances:
    let pred = |report: &FunderReport<_>| {
        let friend = report.friends.get(&public_keys[1]).unwrap();
        let tc_report = match &friend.channel_status {
            ChannelStatusReport::Consistent(tc_report) => tc_report,
            _ => return false,
        };
        tc_report.balance.balance == 3
    };
    await!(node_controls[0].recv_until(pred));

    let pred = |report: &FunderReport<_>| {
        let friend = report.friends.get(&public_keys[0]).unwrap();
        let tc_report = match &friend.channel_status {
            ChannelStatusReport::Consistent(tc_report) => tc_report,
            _ => return false,
        };
        tc_report.balance.balance == -3
    };
    await!(node_controls[1].recv_until(pred));
}

#[test]
fn test_funder_basic() {
    let mut thread_pool = ThreadPool::new().unwrap();
    thread_pool.run(task_funder_basic(thread_pool.clone()));
}

async fn task_funder_forward_payment(spawner: impl Spawn + Clone + Send + 'static) {
    /*
     * 0 -- 1 -- 2
     */
    let num_nodes = 3;
    let mut node_controls = await!(create_node_controls(num_nodes, spawner));

    // Create topology:
    // ----------------
    let public_keys = node_controls
        .iter()
        .map(|nc| nc.public_key.clone())
        .collect::<Vec<PublicKey>>();

    // Add friends:
    let relays0 = vec![dummy_relay_address(0)];
    let relays1 = vec![dummy_relay_address(1)];
    let relays2 = vec![dummy_relay_address(2)];
    await!(node_controls[0].add_friend(&public_keys[1], relays1, "node1", 8));
    await!(node_controls[1].add_friend(&public_keys[0], relays0.clone(), "node0", -8));
    await!(node_controls[1].add_friend(&public_keys[2], relays2, "node2", 6));
    await!(node_controls[2].add_friend(&public_keys[1], relays0, "node0", -6));

    // Enable friends:
    await!(node_controls[0].set_friend_status(&public_keys[1], FriendStatus::Enabled));
    await!(node_controls[1].set_friend_status(&public_keys[0], FriendStatus::Enabled));
    await!(node_controls[1].set_friend_status(&public_keys[2], FriendStatus::Enabled));
    await!(node_controls[2].set_friend_status(&public_keys[1], FriendStatus::Enabled));

    // Set remote max debt:
    await!(node_controls[0].set_remote_max_debt(&public_keys[1], 200));
    await!(node_controls[1].set_remote_max_debt(&public_keys[0], 100));
    await!(node_controls[1].set_remote_max_debt(&public_keys[2], 300));
    await!(node_controls[2].set_remote_max_debt(&public_keys[1], 400));

    // Open requests, allowing this route: 0 --> 1 --> 2
    await!(node_controls[1].set_requests_status(&public_keys[0], RequestsStatus::Open));
    await!(node_controls[2].set_requests_status(&public_keys[1], RequestsStatus::Open));

    // Wait until route is ready (Online + Consistent + open requests)
    // Note: We don't need the other direction to be ready, because the request is sent
    // along the following route: 0 --> 1 --> 2
    await!(node_controls[0].wait_until_ready(&public_keys[1]));
    await!(node_controls[1].wait_until_ready(&public_keys[2]));

    // Send credits 0 --> 2
    let user_request_send_funds = UserRequestSendFunds {
        request_id: Uid::from(&[3; UID_LEN]),
        route: FriendsRoute {
            public_keys: vec![
                public_keys[0].clone(),
                public_keys[1].clone(),
                public_keys[2].clone(),
            ],
        },
        invoice_id: InvoiceId::from(&[1; INVOICE_ID_LEN]),
        dest_payment: 20,
    };
    let incoming_control_message = FunderIncomingControl::new(
        Uid::from(&[42; UID_LEN]),
        FunderControl::RequestSendFunds(user_request_send_funds),
    );
    await!(node_controls[0].send(incoming_control_message)).unwrap();
    let response_received = await!(node_controls[0].recv_until_response()).unwrap();
    assert_eq!(response_received.request_id, Uid::from(&[3; UID_LEN]));
    let receipt = match response_received.result {
        ResponseSendFundsResult::Failure(_) => unreachable!(),
        ResponseSendFundsResult::Success(send_funds_receipt) => send_funds_receipt,
    };

    // Send ReceiptAck:
    let receipt_ack = ReceiptAck {
        request_id: Uid::from(&[3; UID_LEN]),
        receipt_signature: receipt.signature.clone(),
    };
    let incoming_control_message = FunderIncomingControl::new(
        Uid::from(&[43; UID_LEN]),
        FunderControl::ReceiptAck(receipt_ack),
    );
    await!(node_controls[0].send(incoming_control_message)).unwrap();

    let pred = |report: &FunderReport<_>| report.num_ready_receipts == 0;
    await!(node_controls[0].recv_until(pred));

    // Make sure that node2 got the credits:
    let pred = |report: &FunderReport<_>| {
        let friend = match report.friends.get(&public_keys[1]) {
            None => return false,
            Some(friend) => friend,
        };
        let tc_report = match &friend.channel_status {
            ChannelStatusReport::Consistent(tc_report) => tc_report,
            _ => return false,
        };
        tc_report.balance.balance == -6 + 20
    };
    await!(node_controls[2].recv_until(pred));
}

#[test]
fn test_funder_forward_payment() {
    let mut thread_pool = ThreadPool::new().unwrap();
    thread_pool.run(task_funder_forward_payment(thread_pool.clone()));
}

async fn task_funder_payment_failure(spawner: impl Spawn + Clone + Send + 'static) {
    /*
     * 0 -- 1 -- 2
     * We will try to send payment from 0 along the route 0 -- 1 -- 2 -- 3,
     * where 3 does not exist. We expect that node 2 will return a failure response.
     */
    let num_nodes = 4;
    let mut node_controls = await!(create_node_controls(num_nodes, spawner));

    // Create topology:
    // ----------------
    let public_keys = node_controls
        .iter()
        .map(|nc| nc.public_key.clone())
        .collect::<Vec<PublicKey>>();

    // Add friends:
    let relays0 = vec![dummy_relay_address(0)];
    let relays1 = vec![dummy_relay_address(1)];
    let relays2 = vec![dummy_relay_address(2)];
    await!(node_controls[0].add_friend(&public_keys[1], relays1.clone(), "node1", 8));
    await!(node_controls[1].add_friend(&public_keys[0], relays0, "node0", -8));
    await!(node_controls[1].add_friend(&public_keys[2], relays2, "node2", 6));
    await!(node_controls[2].add_friend(&public_keys[1], relays1, "node0", -6));

    // Enable friends:
    await!(node_controls[0].set_friend_status(&public_keys[1], FriendStatus::Enabled));
    await!(node_controls[1].set_friend_status(&public_keys[0], FriendStatus::Enabled));
    await!(node_controls[1].set_friend_status(&public_keys[2], FriendStatus::Enabled));
    await!(node_controls[2].set_friend_status(&public_keys[1], FriendStatus::Enabled));

    // Set remote max debt:
    await!(node_controls[0].set_remote_max_debt(&public_keys[1], 200));
    await!(node_controls[1].set_remote_max_debt(&public_keys[0], 100));
    await!(node_controls[1].set_remote_max_debt(&public_keys[2], 300));
    await!(node_controls[2].set_remote_max_debt(&public_keys[1], 400));

    // Open requests, allowing this route: 0 --> 1 --> 2
    await!(node_controls[1].set_requests_status(&public_keys[0], RequestsStatus::Open));
    await!(node_controls[2].set_requests_status(&public_keys[1], RequestsStatus::Open));

    // Wait until route is ready (Online + Consistent + open requests)
    // Note: We don't need the other direction to be ready, because the request is sent
    // along the following route: 0 --> 1 --> 2
    await!(node_controls[0].wait_until_ready(&public_keys[1]));
    await!(node_controls[1].wait_until_ready(&public_keys[2]));

    // Send credits 0 --> 2
    let user_request_send_funds = UserRequestSendFunds {
        request_id: Uid::from(&[3; UID_LEN]),
        route: FriendsRoute {
            public_keys: vec![
                public_keys[0].clone(),
                public_keys[1].clone(),
                public_keys[2].clone(),
                public_keys[3].clone(),
            ],
        },
        invoice_id: InvoiceId::from(&[1; INVOICE_ID_LEN]),
        dest_payment: 20,
    };
    let incoming_control_message = FunderIncomingControl::new(
        Uid::from(&[44; UID_LEN]),
        FunderControl::RequestSendFunds(user_request_send_funds),
    );
    await!(node_controls[0].send(incoming_control_message)).unwrap();
    let response_received = await!(node_controls[0].recv_until_response()).unwrap();
    assert_eq!(response_received.request_id, Uid::from(&[3; UID_LEN]));
    let reporting_public_key = match response_received.result {
        ResponseSendFundsResult::Failure(reporting_public_key) => reporting_public_key,
        ResponseSendFundsResult::Success(_) => unreachable!(),
    };

    assert_eq!(reporting_public_key, public_keys[2]);

    let friend = node_controls[2]
        .report
        .friends
        .get(&public_keys[1])
        .unwrap();
    let tc_report = match &friend.channel_status {
        ChannelStatusReport::Consistent(tc_report) => tc_report,
        _ => unreachable!(),
    };
    assert_eq!(tc_report.balance.balance, -6);
}

#[test]
fn test_funder_payment_failure() {
    let mut thread_pool = ThreadPool::new().unwrap();
    thread_pool.run(task_funder_payment_failure(thread_pool.clone()));
}

/// Test a basic inconsistency between two adjacent nodes
async fn task_funder_inconsistency_basic<S>(spawner: S)
where
    S: Spawn + Clone + Send + 'static,
{
    let num_nodes = 2;
    let mut node_controls = await!(create_node_controls(num_nodes, spawner));

    let public_keys = node_controls
        .iter()
        .map(|nc| nc.public_key.clone())
        .collect::<Vec<PublicKey>>();

    // We set incompatible initial balances (non zero sum) to cause an inconsistency:
    let relays0 = vec![dummy_relay_address(0)];
    let relays1 = vec![dummy_relay_address(1)];
    await!(node_controls[0].add_friend(&public_keys[1], relays1, "node1", 20));
    await!(node_controls[1].add_friend(&public_keys[0], relays0, "node0", -8));

    await!(node_controls[0].set_friend_status(&public_keys[1], FriendStatus::Enabled));
    await!(node_controls[1].set_friend_status(&public_keys[0], FriendStatus::Enabled));

    // Expect inconsistency, together with reset terms:
    let pred = |report: &FunderReport<_>| {
        let friend = report.friends.get(&public_keys[1]).unwrap();
        let channel_inconsistent_report = match &friend.channel_status {
            ChannelStatusReport::Consistent(_) => return false,
            ChannelStatusReport::Inconsistent(channel_inconsistent_report) => {
                channel_inconsistent_report
            }
        };
        if channel_inconsistent_report.local_reset_terms_balance != 20 {
            return false;
        }
        let reset_terms_report = match &channel_inconsistent_report.opt_remote_reset_terms {
            None => return false,
            Some(reset_terms_report) => reset_terms_report,
        };
        reset_terms_report.balance_for_reset == -8
    };
    await!(node_controls[0].recv_until(pred));

    // Resolve inconsistency
    // ---------------------

    // Obtain reset terms:
    let friend = node_controls[0]
        .report
        .friends
        .get(&public_keys[1])
        .unwrap();
    let channel_inconsistent_report = match &friend.channel_status {
        ChannelStatusReport::Consistent(_) => unreachable!(),
        ChannelStatusReport::Inconsistent(channel_inconsistent_report) => {
            channel_inconsistent_report
        }
    };
    let reset_terms_report = match &channel_inconsistent_report.opt_remote_reset_terms {
        Some(reset_terms_report) => reset_terms_report,
        None => unreachable!(),
    };

    let reset_friend_channel = ResetFriendChannel {
        friend_public_key: public_keys[1].clone(),
        reset_token: reset_terms_report.reset_token.clone(), // TODO: Rename reset_token to reset_token?
    };
    let incoming_control_message = FunderIncomingControl::new(
        Uid::from(&[45; UID_LEN]),
        FunderControl::ResetFriendChannel(reset_friend_channel),
    );
    await!(node_controls[0].send(incoming_control_message)).unwrap();

    // Wait until channel is consistent with the correct balance:
    let pred = |report: &FunderReport<_>| {
        let friend = report.friends.get(&public_keys[1]).unwrap();
        let tc_report = match &friend.channel_status {
            ChannelStatusReport::Consistent(tc_report) => tc_report,
            ChannelStatusReport::Inconsistent(_) => return false,
        };
        tc_report.balance.balance == 8
    };
    await!(node_controls[0].recv_until(pred));

    // Wait until channel is consistent with the correct balance:
    let pred = |report: &FunderReport<_>| {
        let friend = report.friends.get(&public_keys[0]).unwrap();
        let tc_report = match &friend.channel_status {
            ChannelStatusReport::Consistent(tc_report) => tc_report,
            ChannelStatusReport::Inconsistent(_) => return false,
        };
        tc_report.balance.balance == -8
    };
    await!(node_controls[1].recv_until(pred));

    // Make sure that we manage to send messages over the token channel after resolving the
    // inconsistency:
    await!(node_controls[0].set_remote_max_debt(&public_keys[1], 200));
    await!(node_controls[1].set_remote_max_debt(&public_keys[0], 300));
}

#[test]
fn test_funder_inconsistency_basic() {
    let mut thread_pool = ThreadPool::new().unwrap();
    thread_pool.run(task_funder_inconsistency_basic(thread_pool.clone()));
}

/// Test setting relay address for local node
async fn task_funder_add_relay(spawner: impl Spawn + Clone + Send + 'static) {
    let num_nodes = 1;
    let mut node_controls = await!(create_node_controls(num_nodes, spawner));

    // Change the node's relay address:
    let named_relay = dummy_named_relay_address(5);
    // Add twice. Second addition should have no effect:
    await!(node_controls[0].add_relay(named_relay.clone()));
    await!(node_controls[0].add_relay(named_relay.clone()));
    // Remove relay:
    await!(node_controls[0].remove_relay(named_relay.public_key));
}

#[test]
fn test_funder_add_relay() {
    let mut thread_pool = ThreadPool::new().unwrap();
    thread_pool.run(task_funder_add_relay(thread_pool.clone()));
}
