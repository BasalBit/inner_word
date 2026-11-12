//! The channel the events of a conversation travel through.

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};

use writer_assistant::{AssistantEvent, event_channel};

fn notice(text: &str) -> AssistantEvent {
    AssistantEvent::Notice {
        text: text.to_string(),
    }
}

/// A waker that counts how often it was woken.
struct Count(AtomicUsize);

impl Wake for Count {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn events_come_out_in_the_order_they_went_in() {
    let (sink, events) = event_channel();
    assert_eq!(events.try_next(), None);
    sink.send(notice("one"));
    sink.send(notice("two"));
    sink.clone().send(notice("three"));
    assert_eq!(events.try_next(), Some(notice("one")));
    assert_eq!(events.drain(), vec![notice("two"), notice("three")]);
    assert!(events.drain().is_empty());
    assert!(!events.is_finished(), "a sink is still there");
}

#[test]
fn the_stream_ends_when_every_sink_is_gone_and_everything_was_read() {
    let (sink, events) = event_channel();
    let second = sink.clone();
    sink.send(notice("last words"));
    drop(sink);
    assert!(!events.is_finished());
    drop(second);
    assert!(!events.is_finished(), "what was sent is still to be read");
    assert_eq!(
        events.wait(Duration::from_secs(1)),
        Some(notice("last words"))
    );
    assert!(events.is_finished());
    // Waiting on an ended stream returns at once.
    let started = Instant::now();
    assert_eq!(events.wait(Duration::from_secs(5)), None);
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn waiting_blocks_until_an_event_arrives_or_the_time_is_up() {
    let (sink, events) = event_channel();
    let started = Instant::now();
    assert_eq!(events.wait(Duration::from_millis(50)), None);
    assert!(started.elapsed() >= Duration::from_millis(45));
    let sender = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        sink.send(notice("from another thread"));
        sink
    });
    assert_eq!(
        events.wait(Duration::from_secs(5)),
        Some(notice("from another thread"))
    );
    let sink = sender.join().unwrap();
    // wait_until collects up to the event that was waited for.
    sink.send(notice("a"));
    sink.send(notice("b"));
    sink.send(notice("c"));
    let (seen, found) = events.wait_until(Duration::from_secs(1), |event| *event == notice("b"));
    assert!(found);
    assert_eq!(seen, vec![notice("a"), notice("b")]);
    let (seen, found) =
        events.wait_until(Duration::from_millis(30), |event| *event == notice("never"));
    assert!(!found);
    assert_eq!(seen, vec![notice("c")]);
}

#[test]
fn an_awaiting_task_is_woken_by_the_next_event_and_by_the_end() {
    let (sink, events) = event_channel();
    let count = Arc::new(Count(AtomicUsize::new(0)));
    let waker = Waker::from(count.clone());
    let mut cx = Context::from_waker(&waker);

    // Nothing there: the task is parked, and woken exactly when an event arrives.
    let mut next = pin!(events.next());
    assert_eq!(next.as_mut().poll(&mut cx), Poll::Pending);
    assert_eq!(count.0.load(Ordering::SeqCst), 0);
    sink.send(notice("wake up"));
    assert_eq!(count.0.load(Ordering::SeqCst), 1);
    assert_eq!(
        next.as_mut().poll(&mut cx),
        Poll::Ready(Some(notice("wake up")))
    );

    // A batch is everything that is waiting, at once.
    sink.send(notice("a"));
    sink.send(notice("b"));
    let mut batch = pin!(events.next_batch());
    assert_eq!(
        batch.as_mut().poll(&mut cx),
        Poll::Ready(Some(vec![notice("a"), notice("b")]))
    );

    // The end of the stream wakes the task too, and it reads `None`.
    let mut batch = pin!(events.next_batch());
    assert_eq!(batch.as_mut().poll(&mut cx), Poll::Pending);
    drop(sink);
    assert_eq!(count.0.load(Ordering::SeqCst), 2);
    assert_eq!(batch.as_mut().poll(&mut cx), Poll::Ready(None));
    let mut next = pin!(events.next());
    assert_eq!(next.as_mut().poll(&mut cx), Poll::Ready(None));
}

#[test]
fn a_great_many_events_from_several_threads_all_arrive() {
    let (sink, events) = event_channel();
    let senders: Vec<_> = (0..4)
        .map(|thread| {
            let sink = sink.clone();
            std::thread::spawn(move || {
                for index in 0..2500 {
                    sink.send(notice(&format!("{thread}:{index}")));
                }
            })
        })
        .collect();
    drop(sink);
    let mut seen = Vec::new();
    while let Some(event) = events.wait(Duration::from_secs(10)) {
        seen.push(event);
    }
    for sender in senders {
        sender.join().unwrap();
    }
    assert_eq!(seen.len(), 10_000);
    // Each thread's events are in its own order.
    for thread in 0..4 {
        let own: Vec<usize> = seen
            .iter()
            .filter_map(|event| match event {
                AssistantEvent::Notice { text } => text
                    .strip_prefix(&format!("{thread}:"))
                    .map(|n| n.parse().unwrap()),
                _ => None,
            })
            .collect();
        assert_eq!(own.len(), 2500);
        assert!(own.windows(2).all(|pair| pair[0] < pair[1]));
    }
    assert!(events.is_finished());
}
