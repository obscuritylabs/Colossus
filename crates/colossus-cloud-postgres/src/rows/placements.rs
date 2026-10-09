//! Native identity mappings, admission slots and enrollment receipts.
use super::{DomainRow, Metadata, mismatch, set, signed, unsigned};
use colossus_cloud::{
    CloudResult, Enrollment,
    storage::{Admission, CertificateRenewal, EntityValue},
};
use colossus_ports::StoreError;
use diesel::{
    QueryableByName,
    sql_types::{Array, BigInt, Jsonb, Nullable, Text},
};
use serde_json::Value;

row!(TaskReferenceRow { task_id: String => Text });
impl DomainRow for TaskReferenceRow {
    fn from_value(metadata: Metadata, value: &EntityValue) -> Result<Self, StoreError> {
        let EntityValue::Reference(value) = value else {
            return Err(mismatch());
        };
        Ok(Self {
            metadata,
            task_id: value.clone(),
        })
    }
    fn into_value(self) -> CloudResult<EntityValue> {
        Ok(EntityValue::Reference(self.task_id))
    }
}

row!(SessionRow { thread_id: String => Text });
impl DomainRow for SessionRow {
    fn from_value(metadata: Metadata, value: &EntityValue) -> Result<Self, StoreError> {
        let EntityValue::Reference(value) = value else {
            return Err(mismatch());
        };
        Ok(Self {
            metadata,
            thread_id: value.clone(),
        })
    }
    fn into_value(self) -> CloudResult<EntityValue> {
        Ok(EntityValue::Reference(self.thread_id))
    }
}

row!(AdmissionRow { active_tasks: Vec<String> => Array<Text> });
impl DomainRow for AdmissionRow {
    fn from_value(metadata: Metadata, value: &EntityValue) -> Result<Self, StoreError> {
        let EntityValue::Admission(value) = value else {
            return Err(mismatch());
        };
        Ok(Self {
            metadata,
            active_tasks: value.active.iter().cloned().collect(),
        })
    }
    fn into_value(self) -> CloudResult<EntityValue> {
        Ok(Admission {
            active: set(self.active_tasks)?,
        }
        .into())
    }
}

row!(InvitationRow {
    node_id: String => Text,
    label: String => Text,
    roles: Vec<String> => Array<Text>,
    expires_at: i64 => BigInt,
    redeemed_certificate: Option<String> => Nullable<Text>,
    redeemed_csr: Option<String> => Nullable<Text>,
    certificate_pem: Option<String> => Nullable<Text>,
});
impl DomainRow for InvitationRow {
    fn from_value(metadata: Metadata, value: &EntityValue) -> Result<Self, StoreError> {
        let EntityValue::Invitation(value) = value else {
            return Err(mismatch());
        };
        Ok(Self {
            metadata,
            node_id: value.node_id.clone(),
            label: value.label.clone(),
            roles: value.roles.iter().cloned().collect(),
            expires_at: signed(value.expires_at)?,
            redeemed_certificate: value.redeemed_certificate.clone(),
            redeemed_csr: value.redeemed_csr.clone(),
            certificate_pem: value.certificate_pem.clone(),
        })
    }
    fn into_value(self) -> CloudResult<EntityValue> {
        Ok(Enrollment {
            token_hash: self.metadata.id,
            project_id: self.metadata.project_id,
            node_id: self.node_id,
            label: self.label,
            roles: set(self.roles)?,
            expires_at: unsigned(self.expires_at)?,
            redeemed_certificate: self.redeemed_certificate,
            redeemed_csr: self.redeemed_csr,
            certificate_pem: self.certificate_pem,
        }
        .into())
    }
}

row!(RenewalRow {
    renewal_id: String => Text,
    previous_fingerprint: String => Text,
    csr_sha256: String => Text,
    certificate_pem: String => Text,
    certificate_sha256: String => Text,
    issued_at: i64 => BigInt,
});
impl DomainRow for RenewalRow {
    fn from_value(metadata: Metadata, value: &EntityValue) -> Result<Self, StoreError> {
        let EntityValue::Renewal(value) = value else {
            return Err(mismatch());
        };
        Ok(Self {
            metadata,
            renewal_id: value.id.clone(),
            previous_fingerprint: value.previous_fingerprint.clone(),
            csr_sha256: value.csr_sha256.clone(),
            certificate_pem: value.certificate_pem.clone(),
            certificate_sha256: value.certificate_sha256.clone(),
            issued_at: signed(value.issued_at)?,
        })
    }
    fn into_value(self) -> CloudResult<EntityValue> {
        Ok(CertificateRenewal {
            id: self.renewal_id,
            previous_fingerprint: self.previous_fingerprint,
            csr_sha256: self.csr_sha256,
            certificate_pem: self.certificate_pem,
            certificate_sha256: self.certificate_sha256,
            issued_at: unsigned(self.issued_at)?,
        }
        .into())
    }
}

// These operational authorization envelopes intentionally remain opaque JSONB.
row!(AuthFlowRow { record: Value => Jsonb });
impl DomainRow for AuthFlowRow {
    fn from_value(metadata: Metadata, value: &EntityValue) -> Result<Self, StoreError> {
        let EntityValue::AuthFlow(value) = value else {
            return Err(mismatch());
        };
        if value.is_null() {
            return Err(StoreError::Adapter(
                "cloud authorization envelope invalid".into(),
            ));
        }
        Ok(Self {
            metadata,
            record: value.clone(),
        })
    }
    fn into_value(self) -> CloudResult<EntityValue> {
        Ok(EntityValue::AuthFlow(self.record))
    }
}
