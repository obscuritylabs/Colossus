//! Persisted hash encoding independent of serde_json's feature-unified map order.
use colossus_ports::StoreError;
use serde::{
    Serialize, Serializer,
    ser::{SerializeMap, SerializeSeq},
};
use serde_json::Value;

struct SortedValue<'a>(&'a Value);
impl Serialize for SortedValue<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            Value::Object(object) => {
                let mut keys: Vec<_> = object.keys().collect();
                keys.sort_unstable();
                let mut map = serializer.serialize_map(Some(object.len()))?;
                for key in keys {
                    map.serialize_entry(key, &SortedValue(&object[key]))?;
                }
                map.end()
            }
            Value::Array(array) => {
                let mut sequence = serializer.serialize_seq(Some(array.len()))?;
                for value in array {
                    sequence.serialize_element(&SortedValue(value))?;
                }
                sequence.end()
            }
            Value::Number(number)
                if number.is_f64()
                    && number
                        .as_f64()
                        .is_some_and(|value| value == 0.0 && value.is_sign_negative()) =>
            {
                0.0_f64.serialize(serializer)
            }
            // Keep exact integer and floating-number representations. In particular,
            // never convert u64 revisions/sequences through an IEEE-754 value.
            value => value.serialize(serializer),
        }
    }
}
pub(super) fn bytes(value: &Value) -> Result<Vec<u8>, StoreError> {
    serde_json::to_vec(&SortedValue(value))
        .map_err(|_| StoreError::Adapter("cloud value serialization failed".into()))
}
