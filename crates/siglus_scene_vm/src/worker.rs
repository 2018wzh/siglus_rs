use std::{ops::Deref, sync::mpsc::Receiver, thread::JoinHandle};

/// Keeps the decoder receiver and worker in one lifecycle. Disconnect the
/// receiver before joining so a producer blocked on a bounded send can leave.
pub(super) struct JoinedReceiver<T> {
    receiver: Option<Receiver<T>>,
    worker: Option<JoinHandle<()>>,
}
impl<T> JoinedReceiver<T> {
    pub fn disconnect(&mut self) { drop(self.receiver.take()); }
    pub fn is_finished(&self) -> bool { self.worker.as_ref().is_none_or(|worker| worker.is_finished()) }
    pub fn new(receiver: Receiver<T>, worker: JoinHandle<()>) -> Self {
        Self { receiver: Some(receiver), worker: Some(worker) }
    }
    #[cfg(test)]
    pub fn completed(receiver: Receiver<T>) -> Self {
        Self { receiver: Some(receiver), worker: None }
    }
    pub fn shutdown(&mut self) -> std::thread::Result<()> {
        drop(self.receiver.take());
        if let Some(worker) = self.worker.take() { worker.join()?; }
        Ok(())
    }
}
impl<T> Deref for JoinedReceiver<T> {
    type Target = Receiver<T>;
    fn deref(&self) -> &Self::Target { self.receiver.as_ref().expect("live native receiver") }
}
impl<T> Drop for JoinedReceiver<T> {
    fn drop(&mut self) {
        if self.shutdown().is_err() { log::error!("ASTRA_SIGLUS_WORKER_PANIC"); }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shutdown_disconnects_full_channel_before_join() {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let (ready, started) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || { tx.send(1).unwrap(); ready.send(()).unwrap(); let _ = tx.send(2); });
        started.recv().unwrap();
        let mut owner = JoinedReceiver::new(rx, worker);
        owner.shutdown().unwrap();
    }
}
