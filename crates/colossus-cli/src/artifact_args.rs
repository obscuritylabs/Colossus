use super::*;

#[derive(Args)]
pub(super) struct ArtifactsCommand {
    #[command(subcommand)]
    pub(super) command: ArtifactsAction,
}

#[derive(Subcommand)]
pub(super) enum ArtifactsAction {
    /// Upload one policy-authorized bounded workspace file.
    Upload {
        /// Workspace-relative or explicitly policy-authorized file.
        path: PathBuf,
        /// Intended artifact use.
        #[arg(long, value_enum, default_value_t = ArtifactPurposeArg::RunInput)]
        purpose: ArtifactPurposeArg,
        /// Optional stable caller-selected replay key.
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Show caller-owned released artifact metadata.
    Show {
        /// Exact opaque artifact identifier.
        artifact_id: String,
    },
    /// Download one caller-owned artifact through the filesystem policy boundary.
    Download {
        /// Exact opaque artifact identifier.
        artifact_id: String,
        /// Workspace-relative or explicitly policy-authorized destination.
        output: PathBuf,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(super) enum ArtifactPurposeArg {
    RunInput,
    RunOutput,
    Workflow,
    Extension,
    Archive,
}

impl From<ArtifactPurposeArg> for colossus_api::ArtifactPurpose {
    fn from(value: ArtifactPurposeArg) -> Self {
        match value {
            ArtifactPurposeArg::RunInput => Self::RunInput,
            ArtifactPurposeArg::RunOutput => Self::RunOutput,
            ArtifactPurposeArg::Workflow => Self::Workflow,
            ArtifactPurposeArg::Extension => Self::Extension,
            ArtifactPurposeArg::Archive => Self::Archive,
        }
    }
}
