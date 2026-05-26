use criterion::{Criterion, criterion_group, criterion_main};
use synapse_hid_host::{
    HOST_COMMAND_MOUSE_MOVE_REL, MAX_FRAME_LEN, MAX_PAYLOAD_LEN, encode_host_frame,
};

const ONE_MIB: usize = 1024 * 1024;

fn bench_hid_protocol_encode_1mb(c: &mut Criterion) {
    let payload = vec![0x5Au8; ONE_MIB];
    let mut frame = [0u8; MAX_FRAME_LEN];

    c.bench_function("hid_protocol_encode_1mb", |b| {
        b.iter(|| {
            let mut seq = 1u32;
            let mut encoded_bytes = 0usize;

            for chunk in payload.chunks(MAX_PAYLOAD_LEN) {
                let len =
                    match encode_host_frame(seq, HOST_COMMAND_MOUSE_MOVE_REL, chunk, &mut frame) {
                        Ok(len) => len,
                        Err(error) => panic!("benchmark frame encode failed: {error:?}"),
                    };
                encoded_bytes += len;
                seq = seq.wrapping_add(1);
            }

            encoded_bytes
        });
    });
}

criterion_group!(benches, bench_hid_protocol_encode_1mb);
criterion_main!(benches);
