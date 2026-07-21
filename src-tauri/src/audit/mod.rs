//! Audit + finish-check pipelines.

pub mod engine;
pub mod finish_check;

pub use engine::{
    audit, audit_project, audit_with_cuts, audit_with_project, Code, Finding, Report, Severity,
};
pub use finish_check::{
    finish_check_emit, finish_check_emit_for_project, finish_check_emit_with_head,
    finish_check_emit_with_project, finish_check_fix, EmitItem, FinishCheck,
};
