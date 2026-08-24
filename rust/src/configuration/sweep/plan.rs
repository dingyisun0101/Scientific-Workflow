//! Allocation-stable selection over precomputed mixed-radix dimensions.

use super::super::error::ConfigurationError;
use super::super::parameter_path::ParameterPath;
use super::super::parameter_tree::ParameterLeaf;

#[derive(Clone)]
pub(crate) struct SweepPlan {
    dimensions: Vec<SweepDimension>,
    combination_count: u64,
    selectable_paths: Vec<ParameterPath>,
}

#[derive(Clone)]
pub(crate) struct SweepDimension {
    pub(super) label: String,
    pub(super) candidates: Vec<Vec<ParameterLeaf>>,
    pub(super) stride: u64,
}

impl SweepPlan {
    pub(crate) fn new(mut dimensions: Vec<SweepDimension>) -> Result<Self, ConfigurationError> {
        let mut combination_count = 1_u64;
        for dimension in dimensions.iter_mut().rev() {
            dimension.stride = combination_count;
            let length = u64::try_from(dimension.candidates.len()).map_err(|_| {
                ConfigurationError::CombinationCountOverflow {
                    axis: dimension.label.clone(),
                }
            })?;
            combination_count = combination_count.checked_mul(length).ok_or_else(|| {
                ConfigurationError::CombinationCountOverflow {
                    axis: dimension.label.clone(),
                }
            })?;
        }
        let mut selectable_paths = Vec::new();
        for dimension in &dimensions {
            for leaf in &dimension.candidates[0] {
                if !selectable_paths.contains(&leaf.path) {
                    selectable_paths.push(leaf.path.clone());
                }
            }
        }
        Ok(Self {
            dimensions,
            combination_count,
            selectable_paths,
        })
    }

    pub(crate) const fn combination_count(&self) -> u64 {
        self.combination_count
    }

    pub(crate) fn selectable_paths(&self) -> &[ParameterPath] {
        &self.selectable_paths
    }

    pub(crate) fn selected_leaves(&self, ordinal: u64) -> impl Iterator<Item = &ParameterLeaf> {
        self.dimensions.iter().flat_map(move |dimension| {
            let length = dimension.candidates.len() as u64;
            let selected = ((ordinal / dimension.stride) % length) as usize;
            dimension.candidates[selected].iter()
        })
    }
}
