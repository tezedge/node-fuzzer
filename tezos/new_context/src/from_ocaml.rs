// Copyright {c} SimpleStaking and Tezedge Contributors
// SPDX-License-Identifier: MIT

use ocaml_interop::{
    impl_from_ocaml_polymorphic_variant, impl_from_ocaml_variant, OCamlBytes, OCamlFloat, OCamlInt,
    OCamlInt64, OCamlList,
};

use crate::{actions::ContextAction, working_tree::working_tree::FoldDepth};

impl_from_ocaml_variant! {
    ContextAction {
        ContextAction::Set {
            context_hash: Option<OCamlBytes>,
            block_hash: Option<OCamlBytes>,
            operation_hash: Option<OCamlBytes>,
            tree_hash: Option<OCamlBytes>,
            new_tree_hash: Option<OCamlBytes>,
            tree_id: OCamlInt,
            new_tree_id: OCamlInt,
            start_time: OCamlFloat,
            end_time: OCamlFloat,
            key: OCamlList<OCamlBytes>,
            value: OCamlBytes,
            value_as_json: Option<OCamlBytes>,
        },
        ContextAction::Delete {
            context_hash: Option<OCamlBytes>,
            block_hash: Option<OCamlBytes>,
            operation_hash: Option<OCamlBytes>,
            tree_hash: Option<OCamlBytes>,
            new_tree_hash: Option<OCamlBytes>,
            tree_id: OCamlInt,
            new_tree_id: OCamlInt,
            start_time: OCamlFloat,
            end_time: OCamlFloat,
            key: OCamlList<OCamlBytes>,
        },
        ContextAction::RemoveRecursively {
            context_hash: Option<OCamlBytes>,
            block_hash: Option<OCamlBytes>,
            operation_hash: Option<OCamlBytes>,
            tree_hash: Option<OCamlBytes>,
            new_tree_hash: Option<OCamlBytes>,
            tree_id: OCamlInt,
            new_tree_id: OCamlInt,
            start_time: OCamlFloat,
            end_time: OCamlFloat,
            key: OCamlList<OCamlBytes>,
        },
        ContextAction::Copy {
            context_hash: Option<OCamlBytes>,
            block_hash: Option<OCamlBytes>,
            operation_hash: Option<OCamlBytes>,
            tree_hash: Option<OCamlBytes>,
            new_tree_hash: Option<OCamlBytes>,
            tree_id: OCamlInt,
            new_tree_id: OCamlInt,
            start_time: OCamlFloat,
            end_time: OCamlFloat,
            from_key: OCamlList<OCamlBytes>,
            to_key: OCamlList<OCamlBytes>,
        },
        ContextAction::Checkout {
            context_hash: OCamlBytes,
            start_time: OCamlFloat,
            end_time: OCamlFloat,
        },
        ContextAction::Commit {
            parent_context_hash: Option<OCamlBytes>,
            block_hash: Option<OCamlBytes>,
            new_context_hash: OCamlBytes,
            tree_hash: Option<OCamlBytes>,
            tree_id: OCamlInt,
            start_time: OCamlFloat,
            end_time: OCamlFloat,
            author: OCamlBytes,
            message: OCamlBytes,
            date: OCamlInt64,
            parents: OCamlList<OCamlBytes>,
        },
        ContextAction::Mem {
            context_hash: Option<OCamlBytes>,
            block_hash: Option<OCamlBytes>,
            operation_hash: Option<OCamlBytes>,
            tree_hash: Option<OCamlBytes>,
            tree_id: OCamlInt,
            start_time: OCamlFloat,
            end_time: OCamlFloat,
            key: OCamlList<OCamlBytes>,
            value: bool,
        },
        ContextAction::DirMem {
            context_hash: Option<OCamlBytes>,
            block_hash: Option<OCamlBytes>,
            operation_hash: Option<OCamlBytes>,
            tree_hash: Option<OCamlBytes>,
            tree_id: OCamlInt,
            start_time: OCamlFloat,
            end_time: OCamlFloat,
            key: OCamlList<OCamlBytes>,
            value: bool,
        },
        ContextAction::Get {
            context_hash: Option<OCamlBytes>,
            block_hash: Option<OCamlBytes>,
            operation_hash: Option<OCamlBytes>,
            tree_hash: Option<OCamlBytes>,
            tree_id: OCamlInt,
            start_time: OCamlFloat,
            end_time: OCamlFloat,
            key: OCamlList<OCamlBytes>,
            value: OCamlBytes,
            value_as_json: Option<OCamlBytes>,
        },
        ContextAction::Fold {
            context_hash: Option<OCamlBytes>,
            block_hash: Option<OCamlBytes>,
            operation_hash: Option<OCamlBytes>,
            tree_hash: Option<OCamlBytes>,
            tree_id: OCamlInt,
            start_time: OCamlFloat,
            end_time: OCamlFloat,
            key: OCamlList<OCamlBytes>,
        },
    }
}

impl_from_ocaml_polymorphic_variant! {
    FoldDepth {
        Eq(n: OCamlInt) => FoldDepth::Eq(n),
        Le(n: OCamlInt) => FoldDepth::Le(n),
        Lt(n: OCamlInt) => FoldDepth::Lt(n),
        Ge(n: OCamlInt) => FoldDepth::Ge(n),
        Gt(n: OCamlInt) => FoldDepth::Gt(n),
    }
}
