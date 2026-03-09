pub(crate) mod call_targets;
pub(crate) mod capture;
mod context;
pub(crate) mod languages {
    pub(crate) mod go;
    pub(crate) mod java;
    pub(crate) mod javascript;
    pub(crate) mod python;
}

pub(crate) use context::{ParseContext, PythonPropertyCallers, PythonPropertyDefinitions};
