//! Bounded FIFO preserves native focus, raw key, Unicode commit and release order.
use super::Event;
use tokio::sync::mpsc;

// AppKit can deliver a complete key/text/up burst before the asynchronous
// authenticated acknowledgements drain it. The bound remains finite; leases
// are revalidated when each event is actually consumed.
pub(super) const CAPACITY: usize = 256;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Rejection {
    Full,
    Closed,
}
pub(super) fn enqueue(sender: &mpsc::Sender<Event>, event: Event) -> Result<(), Rejection> {
    sender.try_send(event).map_err(|error| match error {
        mpsc::error::TrySendError::Full(_) => Rejection::Full,
        mpsc::error::TrySendError::Closed(_) => Rejection::Closed,
    })
}
pub(super) fn trace(kind: &'static str, error: Rejection) {
    #[cfg(all(target_os = "macos", feature = "native-test-driver", debug_assertions))]
    eprintln!(
        "EVIDENCE mac-native-input stage={kind}_queue reason={}",
        match error {
            Rejection::Full => "full",
            Rejection::Closed => "closed",
        }
    );
    #[cfg(not(all(target_os = "macos", feature = "native-test-driver", debug_assertions)))]
    let _ = (kind, error);
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_browser_presentation::{Input, Lease};
    fn lease() -> Lease {
        Lease {
            tab: 1,
            session_generation: 1,
            control_generation: 0,
            viewport_generation: 2,
            document_generation: 1,
            pixel_width: 2004,
            pixel_height: 1280,
        }
    }
    fn keys() -> Vec<Input> {
        "Mac input café😀x"
            .chars()
            .enumerate()
            .flat_map(|(index, character)| {
                let code = 65 + u16::try_from(index).expect("short input fixture");
                [
                    Input::Key {
                        code,
                        pressed: true,
                    },
                    Input::ImeCommit {
                        text: character.to_string(),
                    },
                    Input::Key {
                        code,
                        pressed: false,
                    },
                ]
            })
            .collect()
    }
    #[test]
    fn ordinary_mac_key_text_release_burst_exceeds32_and_preserves_fifo_under256() {
        let inputs = keys();
        assert!(inputs.len() > 32);
        let (small, _reader) = mpsc::channel(32);
        enqueue(&small, Event::Focus(lease(), true)).unwrap();
        assert!(
            inputs
                .iter()
                .cloned()
                .any(|input| enqueue(&small, Event::Input(lease(), input, 0))
                    == Err(Rejection::Full))
        );
        let (sender, mut reader) = mpsc::channel(CAPACITY);
        enqueue(&sender, Event::Focus(lease(), true)).unwrap();
        for input in &inputs {
            enqueue(&sender, Event::Input(lease(), input.clone(), 0)).unwrap();
        }
        assert!(matches!(reader.try_recv().unwrap(),Event::Focus(actual,true) if actual==lease()));
        for expected in inputs {
            assert!(
                matches!(reader.try_recv().unwrap(),Event::Input(actual,input,0)
                if actual==lease()&&input==expected)
            );
        }
        assert!(reader.try_recv().is_err());
    }
    #[test]
    fn actual_overflow_and_closed_receiver_remain_explicit_rejections() {
        let (sender, mut reader) = mpsc::channel(CAPACITY);
        for _ in 0..CAPACITY {
            enqueue(&sender, Event::Focus(lease(), false)).unwrap();
        }
        assert_eq!(
            enqueue(&sender, Event::Focus(lease(), true)),
            Err(Rejection::Full)
        );
        assert!(matches!(reader.try_recv().unwrap(), Event::Focus(_, false)));
        drop(reader);
        assert_eq!(
            enqueue(&sender, Event::Focus(lease(), true)),
            Err(Rejection::Closed)
        );
    }
}
