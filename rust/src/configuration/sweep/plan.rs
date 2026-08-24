//! Allocation-stable sweep selection over precomputed mixed-radix strides.

use super::super::parameter_path::ParameterPath;
use super::super::parameter_tree::ParameterLeaf;

pub(crate) struct SweepPlan {
    kind: SweepKind,
    selectable_paths: Vec<ParameterPath>,
}

enum SweepKind {
    Cartesian {
        dimensions: Vec<SweepDimension>,
        combination_count: u64,
    },
    Cases {
        cases: Vec<Vec<ParameterLeaf>>,
    },
}

pub(super) struct SweepDimension {
    pub(super) path: ParameterPath,
    pub(super) candidates: Vec<Vec<ParameterLeaf>>,
    pub(super) stride: u64,
}

impl SweepPlan {
    pub(super) fn cartesian(
        dimensions: Vec<SweepDimension>,
        combination_count: u64,
        selectable_paths: Vec<ParameterPath>,
    ) -> Self {
        Self {
            kind: SweepKind::Cartesian {
                dimensions,
                combination_count,
            },
            selectable_paths,
        }
    }

    pub(super) fn cases(
        cases: Vec<Vec<ParameterLeaf>>,
        selectable_paths: Vec<ParameterPath>,
    ) -> Self {
        Self {
            kind: SweepKind::Cases { cases },
            selectable_paths,
        }
    }

    pub(crate) fn combination_count(&self) -> u64 {
        match &self.kind {
            SweepKind::Cartesian {
                combination_count, ..
            } => *combination_count,
            SweepKind::Cases { cases } => {
                u64::try_from(cases.len()).expect("validated case count fits u64")
            }
        }
    }

    pub(crate) fn selectable_paths(&self) -> &[ParameterPath] {
        &self.selectable_paths
    }

    pub(crate) fn all_leaf_paths(&self) -> Box<dyn Iterator<Item = &ParameterPath> + '_> {
        match &self.kind {
            SweepKind::Cartesian { dimensions, .. } => Box::new(
                dimensions
                    .iter()
                    .flat_map(|dimension| dimension.candidates.iter())
                    .flat_map(|candidate| candidate.iter().map(|leaf| &leaf.path)),
            ),
            SweepKind::Cases { cases } => Box::new(
                cases
                    .iter()
                    .flat_map(|case| case.iter().map(|leaf| &leaf.path)),
            ),
        }
    }

    pub(crate) fn selected_leaves(
        &self,
        ordinal: u64,
    ) -> Box<dyn Iterator<Item = &ParameterLeaf> + '_> {
        match &self.kind {
            SweepKind::Cartesian { dimensions, .. } => {
                Box::new(dimensions.iter().flat_map(move |dimension| {
                    let length = dimension.candidates.len() as u64;
                    let selected = ((ordinal / dimension.stride) % length) as usize;
                    dimension.candidates[selected].iter()
                }))
            }
            SweepKind::Cases { cases } => Box::new(cases[ordinal as usize].iter()),
        }
    }

    pub(crate) fn selected_leaf(
        &self,
        ordinal: u64,
        path: &ParameterPath,
    ) -> Option<&ParameterLeaf> {
        self.selected_leaves(ordinal)
            .find(|leaf| &leaf.path == path)
    }

    pub(crate) fn selected_leaf_count(&self, ordinal: u64) -> usize {
        self.selected_leaves(ordinal).count()
    }
}
