use crate::agent_run::{MAX_CREATE_INPUT_PARTS, MAX_RUN_STATUS_FILTERS};
use http_body::Frame;
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tonic::{
    Status,
    body::Body as TonicBody,
    codegen::{Body as HttpBody, Bytes, http::Request},
};
use tower::{Layer, Service};

const CREATE_RUN_PATH: &str = "/colossus.api.v1alpha1.AgentRunService/CreateRun";
const LIST_RUNS_PATH: &str = "/colossus.api.v1alpha1.AgentRunService/ListRuns";
const GRPC_HEADER_BYTES: usize = 5;
const INPUT_FIELD_NUMBER: u64 = 1;
const CREATE_SELECTED_SKILLS_FIELD_NUMBER: u64 = 5;
const CREATE_PLUGIN_SKILLS_FIELD_NUMBER: u64 = 13;
const LIST_STATUSES_FIELD_NUMBER: u64 = 2;

/// Reject semantically oversized repeated request fields while gRPC frames are read.
///
/// Tonic otherwise asks Prost to decode the complete message before the handler can
/// inspect the repeated-field cardinality. A small protobuf message containing many
/// empty values can therefore allocate a much larger collection. This layer parses
/// only the top-level wire structure and stops the body before Prost sees a 129th
/// create input, any forbidden selected skill, or a tenth list status.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RequestCardinalityLayer;

impl<S> Layer<S> for RequestCardinalityLayer {
    type Service = RequestCardinalityService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequestCardinalityService { inner }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RequestCardinalityService<S> {
    inner: S,
}

impl<S> Service<Request<TonicBody>> for RequestCardinalityService<S>
where
    S: Service<Request<TonicBody>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(context)
    }

    fn call(&mut self, request: Request<TonicBody>) -> Self::Future {
        let policy = match request.uri().path() {
            CREATE_RUN_PATH => RequestPolicy::CreateRun,
            LIST_RUNS_PATH => RequestPolicy::ListRuns,
            _ => return self.inner.call(request),
        };
        let (parts, body) = request.into_parts();
        let body = TonicBody::new(RequestGuardedBody {
            inner: body,
            guard: RequestWireGuard::new(policy),
            failed: false,
        });
        self.inner.call(Request::from_parts(parts, body))
    }
}

struct RequestGuardedBody {
    inner: TonicBody,
    guard: RequestWireGuard,
    failed: bool,
}

impl HttpBody for RequestGuardedBody {
    type Data = Bytes;
    type Error = Status;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        if self.failed {
            return Poll::Ready(None);
        }
        match Pin::new(&mut self.inner).poll_frame(context) {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref()
                    && let Err(error) = self.guard.consume(data)
                {
                    self.failed = true;
                    return Poll::Ready(Some(Err(error)));
                }
                Poll::Ready(Some(Ok(frame)))
            }
            other => other,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.failed || self.inner.is_end_stream()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        self.inner.size_hint()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RequestPolicy {
    CreateRun,
    ListRuns,
}

#[derive(Debug)]
struct RequestWireGuard {
    grpc: GrpcState,
    policy: RequestPolicy,
    inputs: usize,
    plugin_skills: usize,
    statuses: usize,
}

impl RequestWireGuard {
    fn new(policy: RequestPolicy) -> Self {
        Self {
            grpc: GrpcState::default(),
            policy,
            inputs: 0,
            plugin_skills: 0,
            statuses: 0,
        }
    }
}

#[derive(Debug)]
enum GrpcState {
    Header {
        bytes: [u8; GRPC_HEADER_BYTES],
        received: usize,
    },
    Message {
        remaining: usize,
        protobuf: ProtobufState,
    },
}

impl Default for GrpcState {
    fn default() -> Self {
        Self::Header {
            bytes: [0; GRPC_HEADER_BYTES],
            received: 0,
        }
    }
}

#[derive(Debug)]
enum ProtobufState {
    Key(Varint),
    Varint(Varint),
    Fixed(usize),
    Length {
        varint: Varint,
        kind: LengthDelimitedKind,
    },
    Bytes(usize),
    PackedStatuses {
        remaining: usize,
        varint: Varint,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LengthDelimitedKind {
    Bytes,
    PackedStatuses,
}

impl Default for ProtobufState {
    fn default() -> Self {
        Self::Key(Varint::default())
    }
}

#[derive(Debug, Default)]
struct Varint {
    value: u64,
    shift: u32,
    bytes: u8,
}

impl Varint {
    const fn is_empty(&self) -> bool {
        self.bytes == 0
    }

    fn push(&mut self, byte: u8) -> Result<Option<u64>, Status> {
        if self.bytes == 10 || (self.bytes == 9 && byte > 1) {
            return Err(malformed_request());
        }
        self.value |= u64::from(byte & 0x7f) << self.shift;
        self.bytes = self.bytes.saturating_add(1);
        if byte & 0x80 == 0 {
            let value = self.value;
            *self = Self::default();
            return Ok(Some(value));
        }
        self.shift = self.shift.saturating_add(7);
        Ok(None)
    }
}

impl RequestWireGuard {
    fn consume(&mut self, mut bytes: &[u8]) -> Result<(), Status> {
        while !bytes.is_empty() {
            match &mut self.grpc {
                GrpcState::Header {
                    bytes: header,
                    received,
                } => {
                    let take = (GRPC_HEADER_BYTES - *received).min(bytes.len());
                    header[*received..*received + take].copy_from_slice(&bytes[..take]);
                    *received += take;
                    bytes = &bytes[take..];
                    if *received == GRPC_HEADER_BYTES {
                        if header[0] != 0 {
                            return Err(Status::unimplemented(
                                "compressed guarded requests are not supported",
                            ));
                        }
                        let length =
                            u32::from_be_bytes([header[1], header[2], header[3], header[4]])
                                as usize;
                        self.grpc = if length == 0 {
                            GrpcState::default()
                        } else {
                            GrpcState::Message {
                                remaining: length,
                                protobuf: ProtobufState::default(),
                            }
                        };
                    }
                }
                GrpcState::Message {
                    remaining,
                    protobuf,
                } => {
                    let available = (*remaining).min(bytes.len());
                    let consumed = consume_protobuf(
                        protobuf,
                        &bytes[..available],
                        remaining,
                        self.policy,
                        &mut self.inputs,
                        &mut self.plugin_skills,
                        &mut self.statuses,
                    )?;
                    *remaining -= consumed;
                    bytes = &bytes[consumed..];
                    if *remaining == 0 {
                        if !matches!(protobuf, ProtobufState::Key(varint) if varint.is_empty()) {
                            return Err(malformed_request());
                        }
                        self.grpc = GrpcState::default();
                    } else if consumed == 0 {
                        return Err(malformed_request());
                    }
                }
            }
        }
        Ok(())
    }
}

fn consume_protobuf(
    state: &mut ProtobufState,
    bytes: &[u8],
    frame_remaining: &usize,
    policy: RequestPolicy,
    inputs: &mut usize,
    plugin_skills: &mut usize,
    statuses: &mut usize,
) -> Result<usize, Status> {
    let mut offset = 0;
    while offset < bytes.len() {
        match state {
            ProtobufState::Key(varint) => {
                let Some(key) = consume_varint(bytes, &mut offset, varint)? else {
                    break;
                };
                let field_number = key >> 3;
                if field_number == 0 {
                    return Err(malformed_request());
                }
                let wire_type = key & 0x07;
                match wire_type {
                    0 => {
                        if policy == RequestPolicy::ListRuns
                            && field_number == LIST_STATUSES_FIELD_NUMBER
                        {
                            increment_statuses(statuses)?;
                        }
                        *state = ProtobufState::Varint(Varint::default());
                    }
                    1 => *state = ProtobufState::Fixed(8),
                    2 => {
                        if policy == RequestPolicy::CreateRun
                            && field_number == CREATE_PLUGIN_SKILLS_FIELD_NUMBER
                        {
                            *plugin_skills = plugin_skills.saturating_add(1);
                            if *plugin_skills > 64 {
                                return Err(Status::resource_exhausted(
                                    "at most 64 qualified plugin skills may be selected",
                                ));
                            }
                        }
                        if policy == RequestPolicy::CreateRun
                            && field_number == CREATE_SELECTED_SKILLS_FIELD_NUMBER
                        {
                            return Err(Status::invalid_argument(
                                "create-run selected_skills is not supported in v1alpha1",
                            ));
                        }
                        if policy == RequestPolicy::CreateRun && field_number == INPUT_FIELD_NUMBER
                        {
                            *inputs = inputs.saturating_add(1);
                            if *inputs > MAX_CREATE_INPUT_PARTS {
                                return Err(Status::resource_exhausted(
                                    "create-run input must contain at most 128 parts",
                                ));
                            }
                        }
                        let kind = if policy == RequestPolicy::ListRuns
                            && field_number == LIST_STATUSES_FIELD_NUMBER
                        {
                            LengthDelimitedKind::PackedStatuses
                        } else {
                            LengthDelimitedKind::Bytes
                        };
                        *state = ProtobufState::Length {
                            varint: Varint::default(),
                            kind,
                        };
                    }
                    5 => *state = ProtobufState::Fixed(4),
                    _ => return Err(malformed_request()),
                }
            }
            ProtobufState::Varint(varint) => {
                if consume_varint(bytes, &mut offset, varint)?.is_some() {
                    *state = ProtobufState::Key(Varint::default());
                } else {
                    break;
                }
            }
            ProtobufState::Fixed(remaining) | ProtobufState::Bytes(remaining) => {
                let take = (*remaining).min(bytes.len() - offset);
                *remaining -= take;
                offset += take;
                if *remaining == 0 {
                    *state = ProtobufState::Key(Varint::default());
                }
            }
            ProtobufState::Length { varint, kind } => {
                let Some(length) = consume_varint(bytes, &mut offset, varint)? else {
                    break;
                };
                let length = usize::try_from(length).map_err(|_| malformed_request())?;
                if length > frame_remaining.saturating_sub(offset) {
                    return Err(malformed_request());
                }
                *state = match (*kind, length) {
                    (_, 0) => ProtobufState::Key(Varint::default()),
                    (LengthDelimitedKind::Bytes, length) => ProtobufState::Bytes(length),
                    (LengthDelimitedKind::PackedStatuses, remaining) => {
                        ProtobufState::PackedStatuses {
                            remaining,
                            varint: Varint::default(),
                        }
                    }
                };
            }
            ProtobufState::PackedStatuses { remaining, varint } => {
                let start = offset;
                let end = offset.saturating_add((*remaining).min(bytes.len() - offset));
                while offset < end {
                    if consume_varint(&bytes[..end], &mut offset, varint)?.is_some() {
                        increment_statuses(statuses)?;
                    }
                }
                *remaining -= offset - start;
                if *remaining == 0 {
                    if !varint.is_empty() {
                        return Err(malformed_request());
                    }
                    *state = ProtobufState::Key(Varint::default());
                }
            }
        }
    }
    Ok(offset)
}

fn increment_statuses(statuses: &mut usize) -> Result<(), Status> {
    *statuses = statuses.saturating_add(1);
    if *statuses > MAX_RUN_STATUS_FILTERS {
        return Err(Status::resource_exhausted(
            "list-runs statuses must contain at most nine values",
        ));
    }
    Ok(())
}

fn consume_varint(
    bytes: &[u8],
    offset: &mut usize,
    varint: &mut Varint,
) -> Result<Option<u64>, Status> {
    while *offset < bytes.len() {
        let byte = bytes[*offset];
        *offset += 1;
        if let Some(value) = varint.push(byte)? {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

fn malformed_request() -> Status {
    Status::invalid_argument("guarded protobuf framing is malformed")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grpc_frame(payload: &[u8]) -> Vec<u8> {
        let mut frame = vec![0];
        frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        frame.extend_from_slice(payload);
        frame
    }

    fn encode_varint(mut value: usize) -> Vec<u8> {
        let mut encoded = Vec::new();
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            encoded.push(byte);
            if value == 0 {
                return encoded;
            }
        }
    }

    fn create_guard() -> RequestWireGuard {
        RequestWireGuard::new(RequestPolicy::CreateRun)
    }

    fn list_guard() -> RequestWireGuard {
        RequestWireGuard::new(RequestPolicy::ListRuns)
    }

    #[test]
    fn rejects_the_129th_input_before_the_message_finishes() {
        let payload = [0x0a, 0x00].repeat(MAX_CREATE_INPUT_PARTS + 1);
        let frame = grpc_frame(&payload);
        let mut guard = create_guard();
        let rejected = guard
            .consume(&frame)
            .expect_err("the 129th input must be rejected");
        assert_eq!(rejected.code(), tonic::Code::ResourceExhausted);
    }

    #[test]
    fn accepts_128_inputs_across_arbitrary_body_fragmentation() {
        let payload = [0x0a, 0x00].repeat(MAX_CREATE_INPUT_PARTS);
        let frame = grpc_frame(&payload);
        let mut guard = create_guard();
        for fragment in frame.chunks(3) {
            guard.consume(fragment).expect("bounded input frame");
        }
        assert_eq!(guard.inputs, MAX_CREATE_INPUT_PARTS);
    }

    #[test]
    fn nested_fields_are_skipped_instead_of_counted_as_top_level_input() {
        let nested = [0x0a, 0x00].repeat(MAX_CREATE_INPUT_PARTS + 1);
        let mut payload = vec![0x12];
        payload.extend_from_slice(&encode_varint(nested.len()));
        payload.extend_from_slice(&nested);
        let mut guard = create_guard();
        for byte in grpc_frame(&payload) {
            guard.consume(&[byte]).expect("nested unknown field");
        }
        assert_eq!(guard.inputs, 0);
    }

    #[test]
    fn rejects_the_first_selected_skill_before_its_value_is_decoded() {
        let frame = grpc_frame(&[0x2a, 0x00]);
        let mut guard = create_guard();
        let mut rejected = None;
        for byte in frame {
            if let Err(error) = guard.consume(&[byte]) {
                rejected = Some(error);
                break;
            }
        }
        let rejected = rejected.expect("selected_skills must be rejected");
        assert_eq!(rejected.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn bounds_plugin_skill_cardinality_before_prost_allocation() {
        let mut accepted = create_guard();
        accepted
            .consume(&grpc_frame(&[0x6a, 0x00].repeat(64)))
            .expect("bounded fields");
        let mut rejected = create_guard();
        assert_eq!(
            rejected
                .consume(&grpc_frame(&[0x6a, 0x00].repeat(65)))
                .expect_err("cardinality")
                .code(),
            tonic::Code::ResourceExhausted
        );
    }

    #[test]
    fn rejects_the_tenth_unpacked_list_status() {
        let payload = [0x10, 0x01].repeat(MAX_RUN_STATUS_FILTERS + 1);
        let mut guard = list_guard();
        let rejected = guard
            .consume(&grpc_frame(&payload))
            .expect_err("the tenth unpacked status must be rejected");
        assert_eq!(rejected.code(), tonic::Code::ResourceExhausted);
    }

    #[test]
    fn rejects_the_tenth_packed_list_status_across_fragmentation() {
        let mut payload = vec![0x12, (MAX_RUN_STATUS_FILTERS + 1) as u8];
        payload.extend(std::iter::repeat_n(1, MAX_RUN_STATUS_FILTERS + 1));
        let mut guard = list_guard();
        let mut rejected = None;
        for byte in grpc_frame(&payload) {
            if let Err(error) = guard.consume(&[byte]) {
                rejected = Some(error);
                break;
            }
        }
        let rejected = rejected.expect("the tenth packed status must be rejected");
        assert_eq!(rejected.code(), tonic::Code::ResourceExhausted);
    }

    #[test]
    fn accepts_nine_list_statuses_across_packed_and_unpacked_fields() {
        let mut payload = vec![0x12, 0x08];
        payload.extend(std::iter::repeat_n(1, 8));
        payload.extend_from_slice(&[0x10, 0x01]);
        let mut guard = list_guard();
        for fragment in grpc_frame(&payload).chunks(2) {
            guard.consume(fragment).expect("bounded status filters");
        }
        assert_eq!(guard.statuses, MAX_RUN_STATUS_FILTERS);
    }
}
