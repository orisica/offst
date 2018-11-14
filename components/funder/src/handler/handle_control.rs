use crypto::identity::PublicKey;
use crypto::crypto_rand::{RandValue, CryptoRandom};

use super::super::friend::{FriendMutation, ChannelStatus};
use super::super::state::{FunderMutation};
use super::{MutableFunderHandler, 
    MAX_MOVE_TOKEN_LENGTH};
use super::super::messages::ResponseSendFundsResult;
use super::super::types::{FriendStatus, UserRequestSendFunds,
    SetFriendRemoteMaxDebt, ResetFriendChannel,
    SetFriendAddr, AddFriend, RemoveFriend, SetFriendStatus, SetRequestsStatus, 
    ReceiptAck, FriendMoveToken, IncomingControlMessage,
    FriendTcOp, ResponseReceived,
    ChannelerConfig, FunderOutgoingComm};
use super::sender::SendMode;

// TODO: Should be an argument of the Funder:
const MAX_PENDING_USER_REQUESTS: usize = 0x10;

#[derive(Debug)]
pub enum HandleControlError {
    FriendDoesNotExist,
    TokenChannelDoesNotExist,
    NotInvitedToReset,
    ResetTokenMismatch,
    NotFirstInRoute,
    InvalidRoute,
    RequestAlreadyInProgress,
    PendingUserRequestsFull,
    ReceiptDoesNotExist,
    UserRequestInvalid,
    FriendNotReady,
    BlockedByFreezeGuard,
}


#[allow(unused)]
impl<A:Clone + 'static, R: CryptoRandom + 'static> MutableFunderHandler<A,R> {

    async fn control_set_friend_remote_max_debt(&mut self, 
                                            set_friend_remote_max_debt: SetFriendRemoteMaxDebt) 
        -> Result<(), HandleControlError> {

        // Make sure that friend exists:
        let friend = self.get_friend(&set_friend_remote_max_debt.friend_public_key)
            .ok_or(HandleControlError::FriendDoesNotExist)?;

        if friend.wanted_remote_max_debt == set_friend_remote_max_debt.remote_max_debt {
            // Wanted remote max debt is already set to this value. Nothing to do here.
            return Ok(())
        }

        // We only set the wanted remote max debt here. The actual remote max debt will be changed
        // only when we manage to send a move token message containing the SetRemoteMaxDebt
        // operation.
        let friend_mutation = FriendMutation::SetWantedRemoteMaxDebt(set_friend_remote_max_debt.remote_max_debt);
        let m_mutation = FunderMutation::FriendMutation(
            (set_friend_remote_max_debt.friend_public_key.clone(), friend_mutation));

        self.apply_mutation(m_mutation);
        await!(self.try_send_channel(&set_friend_remote_max_debt.friend_public_key, SendMode::EmptyNotAllowed));
        Ok(())
    }

    async fn control_reset_friend_channel(&mut self, 
                                    reset_friend_channel: ResetFriendChannel) 
        -> Result<(), HandleControlError> {

        let friend = self.get_friend(&reset_friend_channel.friend_public_key)
            .ok_or(HandleControlError::FriendDoesNotExist)?;

        let remote_reset_terms = match &friend.channel_status {
            ChannelStatus::Consistent(_) => Err(HandleControlError::NotInvitedToReset),
            ChannelStatus::Inconsistent(channel_inconsistent) => {
                match &channel_inconsistent.opt_remote_reset_terms {
                    None => Err(HandleControlError::NotInvitedToReset),
                    Some(remote_reset_terms) => {
                        if (remote_reset_terms.reset_token != reset_friend_channel.current_token)  {
                            Err(HandleControlError::ResetTokenMismatch)
                        } else {
                            Ok(remote_reset_terms)
                        }
                    },
                }
            },
        }?;

        let rand_nonce = RandValue::new(&self.rng);
        let move_token_counter = 0;

        let local_pending_debt = 0;
        let remote_pending_debt = 0;

        let friend_move_token = await!(FriendMoveToken::new(
            // No operations are required for a reset move token
            Vec::new(), 
            remote_reset_terms.reset_token.clone(),
            remote_reset_terms.inconsistency_counter,
            move_token_counter,
            remote_reset_terms.balance_for_reset,
            local_pending_debt,
            remote_pending_debt,
            rand_nonce,
            self.identity_client.clone()));

        await!(self.cancel_local_pending_requests(
            reset_friend_channel.friend_public_key.clone()));

        let friend_mutation = FriendMutation::LocalReset(friend_move_token.clone());
        let m_mutation = FunderMutation::FriendMutation(
            (reset_friend_channel.friend_public_key.clone(), friend_mutation));
        self.apply_mutation(m_mutation);

        self.transmit_outgoing(&reset_friend_channel.friend_public_key);

        Ok(())
    }

    fn enable_friend(&mut self, 
                     friend_public_key: &PublicKey,
                     friend_address: &A) {

        // Notify Channeler:
        let channeler_config = ChannelerConfig::AddFriend(
            (friend_public_key.clone(), friend_address.clone()));
        self.add_outgoing_comm(FunderOutgoingComm::ChannelerConfig(channeler_config));

    }

    fn disable_friend(&mut self, 
                     friend_public_key: &PublicKey) {

        // Notify Channeler:
        let channeler_config = ChannelerConfig::RemoveFriend(
            friend_public_key.clone());
        self.add_outgoing_comm(FunderOutgoingComm::ChannelerConfig(channeler_config));
    }

    fn control_add_friend(&mut self, add_friend: AddFriend<A>) 
        -> Result<(), HandleControlError> {

        let m_mutation = FunderMutation::AddFriend((
                add_friend.friend_public_key.clone(),
                add_friend.address.clone()));

        self.apply_mutation(m_mutation);

        self.enable_friend(&add_friend.friend_public_key,
                           &add_friend.address);

        Ok(())
    }

    /// This is a violent operation, as it removes all the known state with the remote friend.  
    /// An inconsistency will occur if the friend is added again.
    async fn control_remove_friend(&mut self, remove_friend: RemoveFriend) 
        -> Result<(), HandleControlError> {

        // Make sure that friend exists:
        let _friend = self.get_friend(&remove_friend.friend_public_key)
            .ok_or(HandleControlError::FriendDoesNotExist)?;

        self.disable_friend(&remove_friend.friend_public_key);

        await!(self.cancel_local_pending_requests(
            remove_friend.friend_public_key.clone()));
        await!(self.cancel_pending_requests(
                remove_friend.friend_public_key.clone()));
        await!(self.cancel_pending_user_requests(
                remove_friend.friend_public_key.clone()));

        let m_mutation = FunderMutation::RemoveFriend(
                remove_friend.friend_public_key.clone());

        self.apply_mutation(m_mutation);
        Ok(())
    }

    fn control_set_friend_status(&mut self, set_friend_status: SetFriendStatus) 
        -> Result<(), HandleControlError> {

        // Make sure that friend exists:
        let _ = self.get_friend(&set_friend_status.friend_public_key)
            .ok_or(HandleControlError::FriendDoesNotExist)?;

        let friend_mutation = FriendMutation::SetStatus(set_friend_status.status.clone());
        let m_mutation = FunderMutation::FriendMutation(
            (set_friend_status.friend_public_key.clone(), friend_mutation));
        self.apply_mutation(m_mutation);

        let friend = self.get_friend(&set_friend_status.friend_public_key)
            .ok_or(HandleControlError::FriendDoesNotExist)?;

        let friend_public_key = &set_friend_status.friend_public_key;
        let friend_address = friend.remote_address.clone();

        match set_friend_status.status {
            FriendStatus::Enable => self.enable_friend(friend_public_key, &friend_address),
            FriendStatus::Disable => self.disable_friend(&friend_public_key),
        };

        Ok(())
    }

    async fn control_set_requests_status(&mut self, set_requests_status: SetRequestsStatus) 
        -> Result<(), HandleControlError> {

        // Make sure that friend exists:
        let _friend = self.get_friend(&set_requests_status.friend_public_key)
            .ok_or(HandleControlError::FriendDoesNotExist)?;

        let friend_mutation = FriendMutation::SetWantedLocalRequestsStatus(set_requests_status.status);
        let m_mutation = FunderMutation::FriendMutation(
            (set_requests_status.friend_public_key.clone(), friend_mutation));

        self.apply_mutation(m_mutation);
        await!(self.try_send_channel(&set_requests_status.friend_public_key, SendMode::EmptyNotAllowed));
        Ok(())
    }

    fn control_set_friend_addr(&mut self, set_friend_addr: SetFriendAddr<A>) 
        -> Result<(), HandleControlError> {

        // Make sure that friend exists:
        let _friend = self.get_friend(&set_friend_addr.friend_public_key)
            .ok_or(HandleControlError::FriendDoesNotExist)?;

        let friend_mutation = FriendMutation::SetFriendAddr(
            set_friend_addr.address.clone());
        let m_mutation = FunderMutation::FriendMutation(
            (set_friend_addr.friend_public_key.clone(), friend_mutation));

        self.apply_mutation(m_mutation);

        // Notify Channeler to change the friend's address:
        self.disable_friend(&set_friend_addr.friend_public_key);
        self.enable_friend(&set_friend_addr.friend_public_key, 
                           &set_friend_addr.address);

        Ok(())
    }

    fn check_user_request_valid(&self, 
                                user_request_send_funds: &UserRequestSendFunds) 
                                -> Option<()> {

        let friend_op = FriendTcOp::RequestSendFunds(user_request_send_funds.clone().to_request());
        MAX_MOVE_TOKEN_LENGTH.checked_sub(friend_op.approx_bytes_count())?;
        Some(())
    }

    async fn control_request_send_funds_inner(&mut self, user_request_send_funds: UserRequestSendFunds)
        -> Result<(), HandleControlError> {

        self.check_user_request_valid(&user_request_send_funds)
            .ok_or(HandleControlError::UserRequestInvalid)?;

        // If we already have a receipt for this request, we return the receipt immediately and
        // exit. Note that we don't erase the receipt yet. This will only be done when a receipt
        // ack is received.
        if let Some(receipt) = self.state.ready_receipts.get(&user_request_send_funds.request_id) {
            let response_received = ResponseReceived {
                request_id: user_request_send_funds.request_id,
                result: ResponseSendFundsResult::Success(receipt.clone()),
            };
            self.add_response_received(response_received);
            return Ok(());
        }

        let route = &user_request_send_funds.route;

        // We have to be the first on the route:
        match route.public_keys.first() {
            Some(first) if *first == self.state.local_public_key => Ok(()),
            _ => Err(HandleControlError::NotFirstInRoute),
        }?;

        // We want to have at least two public keys on the route (source and destination).
        // We also want that the public keys on the route are unique.
        if (route.len() < 2) || !route.is_cycle_free() {
            return Err(HandleControlError::InvalidRoute);
        }
        let friend_public_key = route.public_keys[1].clone();

        let friend = match self.get_friend(&friend_public_key) {
            Some(friend) => Ok(friend),
            None => Err(HandleControlError::FriendDoesNotExist),
        }?;

        if !self.is_friend_ready(&friend_public_key) {
            return Err(HandleControlError::FriendNotReady);
        }

        // If request is already in progress, we do nothing:
        // Check if there is already a pending user request with the same request_id:
        for user_request in &friend.pending_user_requests {
            if user_request_send_funds.request_id == user_request.request_id {
                return Err(HandleControlError::RequestAlreadyInProgress);
            }
        }

        let token_channel = match &friend.channel_status {
            ChannelStatus::Inconsistent(_) => unreachable!(),
            ChannelStatus::Consistent(token_channel) => token_channel
        };

        // Check if there is an onging request with the same request_id with this specific friend:
        if token_channel
            .get_mutual_credit()
            .state()
            .pending_requests
            .pending_local_requests
            .contains_key(&user_request_send_funds.request_id) {
                return Err(HandleControlError::RequestAlreadyInProgress);
        }

        // Check if we have room to push this message:
        if friend.pending_user_requests.len() >= MAX_PENDING_USER_REQUESTS {
            return Err(HandleControlError::PendingUserRequestsFull);
        }

        let mut request_send_funds = user_request_send_funds.to_request();
        self.add_local_freezing_link(&mut request_send_funds);

        let verify_res = self.ephemeral
            .freeze_guard
            .verify_freezing_links(&request_send_funds.route,
                                   request_send_funds.dest_payment,
                                   &request_send_funds.freeze_links);

        if verify_res.is_none() {
            return Err(HandleControlError::BlockedByFreezeGuard);
        }


        let friend_mutation = FriendMutation::PushBackPendingUserRequest(request_send_funds);
        let funder_mutation = FunderMutation::FriendMutation((friend_public_key.clone(), friend_mutation));
        self.apply_mutation(funder_mutation);
        await!(self.try_send_channel(&friend_public_key, SendMode::EmptyNotAllowed));

        Ok(())
    }


    async fn control_request_send_funds(&mut self, user_request_send_funds: UserRequestSendFunds) 
        -> Result<(), HandleControlError> {
        
        // If we managed to push the message, we return an Ok(()).
        // Otherwise, we return the internal error and return a response failure message.
        await!(self.control_request_send_funds_inner(user_request_send_funds.clone()))
            .map_err(|e| {
                let response_received = ResponseReceived {
                    request_id: user_request_send_funds.request_id,
                    result: ResponseSendFundsResult::Failure(self.state.local_public_key.clone()),
                };
                self.add_response_received(response_received);
                e
            })
    }

    /// Handle an incoming receipt ack message
    fn control_receipt_ack(&mut self, receipt_ack: ReceiptAck) 
        -> Result<(), HandleControlError> {

        if !self.state.ready_receipts.contains_key(&receipt_ack.request_id) {
            return Err(HandleControlError::ReceiptDoesNotExist);
        }
        let m_mutation = FunderMutation::RemoveReceipt(receipt_ack.request_id);
        self.apply_mutation(m_mutation);

        Ok(())
    }


    pub async fn handle_control_message(&mut self, 
                                  funder_config: IncomingControlMessage<A>) 
        -> Result<(), HandleControlError> {


        match funder_config {
            IncomingControlMessage::SetFriendRemoteMaxDebt(set_friend_remote_max_debt) => {
                await!(self.control_set_friend_remote_max_debt(set_friend_remote_max_debt));
            },
            IncomingControlMessage::ResetFriendChannel(reset_friend_channel) => {
                await!(self.control_reset_friend_channel(reset_friend_channel))?;
            },
            IncomingControlMessage::AddFriend(add_friend) => {
                self.control_add_friend(add_friend);
            },
            IncomingControlMessage::RemoveFriend(remove_friend) => {
                await!(self.control_remove_friend(remove_friend))?;
            },
            IncomingControlMessage::SetFriendStatus(set_friend_status) => {
                self.control_set_friend_status(set_friend_status);
            },
            IncomingControlMessage::SetRequestsStatus(set_requests_status) => {
                await!(self.control_set_requests_status(set_requests_status));
            },
            IncomingControlMessage::SetFriendAddr(set_friend_addr) => {
                self.control_set_friend_addr(set_friend_addr);
            },
            IncomingControlMessage::RequestSendFunds(user_request_send_funds) => {
                await!(self.control_request_send_funds(user_request_send_funds));
            },
            IncomingControlMessage::ReceiptAck(receipt_ack) => {
                self.control_receipt_ack(receipt_ack);
            }
        };
        Ok(())
    }
}
