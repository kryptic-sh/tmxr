//! Property tests of the frame codec, as plan 14's fuzzing asks: whatever
//! bytes arrive, reading a message fails cleanly instead of panicking, and
//! what is written reads back as it was.

use proptest::prelude::*;
use tmxr_proto::{ClientMsg, ServerMsg, read_msg, write_msg};

proptest! {
    #[test]
    fn any_bytes_read_as_a_message_or_an_error(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        let _ = read_msg::<_, ClientMsg>(&mut bytes.as_slice());
        let _ = read_msg::<_, ServerMsg>(&mut bytes.as_slice());
    }

    #[test]
    fn a_frame_with_any_body_reads_as_a_message_or_an_error(body in proptest::collection::vec(any::<u8>(), 0..512)) {
        let mut frame = u32::try_from(body.len()).unwrap().to_le_bytes().to_vec();
        frame.extend_from_slice(&body);
        let _ = read_msg::<_, ClientMsg>(&mut frame.as_slice());
        let _ = read_msg::<_, ServerMsg>(&mut frame.as_slice());
    }

    #[test]
    fn a_command_reads_back_as_written(words in proptest::collection::vec(".*", 0..8)) {
        let mut frame = Vec::new();
        write_msg(&mut frame, &ClientMsg::Command(words.clone())).unwrap();
        let read: Option<ClientMsg> = read_msg(&mut frame.as_slice()).unwrap();
        prop_assert!(matches!(read, Some(ClientMsg::Command(w)) if w == words));
    }

    #[test]
    fn output_reads_back_as_written(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let mut frame = Vec::new();
        write_msg(&mut frame, &ServerMsg::Output(bytes.clone())).unwrap();
        let read: Option<ServerMsg> = read_msg(&mut frame.as_slice()).unwrap();
        prop_assert!(matches!(read, Some(ServerMsg::Output(b)) if b == bytes));
    }
}
