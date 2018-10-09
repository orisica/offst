use crypto::identity::PublicKey;

// M: Stream<Item=RelayListenIn, Error=()>,
// K: Sink<SinkItem=RelayListenOut, SinkError=()>,
#[allow(unused)]
pub struct IncomingListen<M,K> {
    pub receiver: M,
    pub sender: K,
}

// M: Stream<Item=TunnelMessage, Error=()>>,
// K: Sink<SinkItem=TunnelMessage, SinkError=()>>,
#[allow(unused)]
pub struct IncomingAccept<M,K> {
    pub receiver: M,
    pub sender: K,
    pub accept_public_key: PublicKey,
}

// M: Stream<Item=TunnelMessage, Error=()>,
// K: SinkItem=TunnelMessage, SinkError=()>,
#[allow(unused)]
pub struct IncomingConnect<M,K> {
    pub receiver: M,
    pub sender: K,
    pub connect_public_key: PublicKey,
}

#[allow(unused)]
pub enum IncomingConnInner<ML,KL,MA,KA,MC,KC> {
    Listen(IncomingListen<ML,KL>),
    Accept(IncomingAccept<MA,KA>),
    Connect(IncomingConnect<MC,KC>),
}

#[allow(unused)]
pub struct IncomingConn<ML,KL,MA,KA,MC,KC> {
    pub public_key: PublicKey,
    pub inner: IncomingConnInner<ML,KL,MA,KA,MC,KC>,
}