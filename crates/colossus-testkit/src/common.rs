use super::*;

pub(super) fn conformance_actor(id: &str) -> Actor {
    Actor {
        actor_type: ActorType::User,
        id: id.into(),
    }
}
