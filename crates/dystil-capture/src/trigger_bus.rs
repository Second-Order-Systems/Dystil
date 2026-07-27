use tokio::sync::broadcast;

/// Dystil-owned broadcast bus for capture triggers.
///
/// The payload remains generic so platform adapters can use their existing
/// event type while channel ownership and lifetime live above visual capture.
pub struct TriggerBus<T: Clone> {
    sender: broadcast::Sender<T>,
}

impl<T: Clone> TriggerBus<T> {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "trigger bus capacity must be non-zero");
        let (sender, _receiver) = broadcast::channel(capacity);
        Self { sender }
    }

    pub fn sender(&self) -> broadcast::Sender<T> {
        self.sender.clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<T> {
        self.sender.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sender_and_subscriber_share_the_owned_channel() {
        let bus = TriggerBus::<u64>::new(8);
        let sender = bus.sender();
        let mut receiver = bus.subscribe();

        sender.send(42).unwrap();

        assert_eq!(receiver.recv().await.unwrap(), 42);
    }
}
