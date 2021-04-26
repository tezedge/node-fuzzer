// Copyright (c) SimpleStaking and Tezedge Contributors
// SPDX-License-Identifier: MIT

//! Temporary bootstrap state which helps to download and validate branches from peers
//!
//! - every peer has his own BootstrapState
//! - bootstrap state is initialized from branch history, which is splitted to partitions
//!
//! - it is king of bingo, where we prepare block intervals, and we check/mark what is downloaded/applied, and what needs to be downloaded or applied

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use riker::actors::*;
use slog::{debug, info, warn, Logger};

use crypto::hash::{BlockHash, ChainId};
use networking::PeerId;
use tezos_messages::p2p::encoding::block_header::Level;

use crate::peer_branch_bootstrapper::PeerBranchBootstrapperRef;
use crate::shell_channel::{ShellChannelMsg, ShellChannelRef, ShellChannelTopic};
use crate::state::data_requester::DataRequester;
use crate::state::peer_state::DataQueues;
use crate::state::synchronization_state::PeerBranchSynchronizationDone;
use crate::state::ApplyBlockBatch;

type BlockStateRef = BlockState;
/// BootstrapState helps to easily manage/mutate inner state
pub struct BootstrapState {
    /// Holds peers info
    peers: HashMap<ActorUri, PeerBootstrapState>,

    /// Holds unique blocks cache, shared for all branch bootstraps to minimalize memory usage
    block_state_db: BlockStateDb,
}

impl Default for BootstrapState {
    fn default() -> Self {
        Self {
            peers: Default::default(),
            block_state_db: BlockStateDb::new(1024),
        }
    }
}

impl BootstrapState {
    pub fn peers_count(&self) -> usize {
        self.peers.len()
    }

    pub fn clean_peer_data(&mut self, peer_actor_uri: &ActorUri) {
        if let Some(mut state) = self.peers.remove(peer_actor_uri) {
            state.branches.clear();
        }
    }

    pub fn check_stalled_peers<DP: Fn(&PeerId)>(
        &mut self,
        _stalled_timeout: Duration,
        missing_branch_bootstrap_timeout: &Duration,
        log: &Logger,
        disconnect_peer: DP,
    ) {
        let stalled_peers = self.peers
            .values()
            .filter(|PeerBootstrapState {empty_bootstrap_state, peer_id, ..}| {
                let mut is_stalled = false;
                if let Some(empty_bootstrap_state) = empty_bootstrap_state.as_ref() {
                    if empty_bootstrap_state.elapsed() > *missing_branch_bootstrap_timeout {
                        warn!(log, "Peer did not sent new curent_head/current_branch for a long time";
                           "peer_id" => peer_id.peer_id_marker.clone(), "peer_ip" => peer_id.peer_address.to_string(), "peer" => peer_id.peer_ref.name(), "peer_uri" => peer_id.peer_ref.uri().to_string());
                        is_stalled = true;
                    }
                }
                is_stalled
            })
            .map(|peer_state| peer_state.peer_id.clone())
            .collect::<Vec<_>>();

        for peer_id in stalled_peers {
            warn!(log, "Disconnecting peer, because of stalled bootstrap pipeline";
                       "peer_id" => peer_id.peer_id_marker.clone(), "peer_ip" => peer_id.peer_address.to_string(), "peer" => peer_id.peer_ref.name(), "peer_uri" => peer_id.peer_ref.uri().to_string());

            self.clean_peer_data(peer_id.peer_ref.uri());
            disconnect_peer(&peer_id);
        }
    }

    pub fn block_apply_failed(&mut self, failed_block: &BlockHash, log: &Logger) {
        self.peers
            .values_mut()
            .for_each(|PeerBootstrapState {branches, peer_id, empty_bootstrap_state, ..}| {
                branches
                    .retain(|branch| {
                        if branch.contains_block(&failed_block) {
                            warn!(log, "Peer's branch bootstrap contains failed block, so this branch bootstrap is removed";
                               "block_hash" => failed_block.to_base58_check(),
                               "to_level" => &branch.to_level,
                               "peer_id" => peer_id.peer_id_marker.clone(), "peer_ip" => peer_id.peer_address.to_string(), "peer" => peer_id.peer_ref.name(), "peer_uri" => peer_id.peer_ref.uri().to_string());
                            false
                        } else {
                            true
                        }
                    });

                if branches.is_empty() && empty_bootstrap_state.is_none() {
                    *empty_bootstrap_state = Some(Instant::now());
                }
            });
    }

    pub fn blocks_stats(&self) -> (usize, (usize, usize, usize, usize)) {
        (
            self.block_state_db.blocks.len(),
            self.block_state_db.blocks.iter().fold(
                (0, 0, 0, 0),
                |(block_downloaded, operations_downloaded, applied, scheduled_for_apply),
                 (_, state)| {
                    (
                        block_downloaded + (if state.block_downloaded { 1 } else { 0 }),
                        operations_downloaded + (if state.operations_downloaded { 1 } else { 0 }),
                        applied + (if state.applied { 1 } else { 0 }),
                        scheduled_for_apply + (if state.scheduled_for_apply { 1 } else { 0 }),
                    )
                },
            ),
        )
    }

    pub fn block_intervals_stats(&self) -> (usize, (usize, usize)) {
        self.peers
            .iter()
            .fold((0, (0, 0)), |stats_acc, (_, peer_state)| {
                peer_state.branches.iter().fold(
                    stats_acc,
                    |(
                        mut intervals_count,
                        (
                            mut intervals_blocks_downloaded,
                            mut intervals_block_operations_downloaded,
                        ),
                    ),
                     branch| {
                        for interval in &branch.intervals {
                            intervals_count += 1;
                            if interval.all_blocks_downloaded {
                                intervals_blocks_downloaded += 1;
                            }
                            if interval.all_operations_downloaded {
                                intervals_block_operations_downloaded += 1;
                            }
                        }
                        (
                            intervals_count,
                            (
                                intervals_blocks_downloaded,
                                intervals_block_operations_downloaded,
                            ),
                        )
                    },
                )
            })
    }
    pub fn add_new_branch(
        &mut self,
        peer_id: Arc<PeerId>,
        peer_queues: Arc<DataQueues>,
        last_applied_block: Arc<BlockHash>,
        missing_history: Vec<Arc<BlockHash>>,
        to_level: Arc<Level>,
        max_bootstrap_branches_per_peer: usize,
        log: &Logger,
    ) {
        // add branch to inner state
        if let Some(peer_state) = self.peers.get_mut(peer_id.peer_ref.uri()) {
            let mut was_merged = false;
            for branch in peer_state.branches.iter_mut() {
                if branch.merge(to_level.clone(), &missing_history) {
                    was_merged = true;
                    peer_state.empty_bootstrap_state = None;
                    break;
                }
            }

            if !was_merged {
                // we handle just finite branches from one peer
                if peer_state.branches.len() >= max_bootstrap_branches_per_peer {
                    debug!(log, "Peer has started already maximum ({}) branch pipelines, so we dont start new one", max_bootstrap_branches_per_peer;
                                    "to_level" => &to_level,
                                    "peer_id" => peer_id.peer_id_marker.clone(), "peer_ip" => peer_id.peer_address.to_string(), "peer" => peer_id.peer_ref.name(), "peer_uri" => peer_id.peer_ref.uri().to_string());
                    return;
                }

                peer_state.branches.push(BranchState::new(
                    last_applied_block,
                    missing_history,
                    to_level,
                ));
                peer_state.empty_bootstrap_state = None;
            }

        } else {
            self.peers.insert(
                peer_id.peer_ref.uri().clone(),
                PeerBootstrapState {
                    peer_id,
                    peer_queues,
                    branches: vec![BranchState::new(
                        last_applied_block,
                        missing_history,
                        to_level,
                    )],
                    empty_bootstrap_state: None,
                    is_bootstrapped: false,
                },
            );

        }
    }

    pub fn schedule_blocks_to_download(&mut self, requester: &DataRequester, log: &Logger) {
        for PeerBootstrapState {
            peer_id,
            peer_queues,
            branches,
            ..
        } in self.peers.values_mut()
        {
            // check peers blocks queue
            let (already_queued, mut available_queue_capacity) = match peer_queues
                .get_already_queued_block_headers_and_max_capacity()
            {
                Ok(queued_and_capacity) => queued_and_capacity,
                Err(e) => {
                    warn!(log, "Failed to get available blocks queue capacity for peer, so ignore this run for peer"; "reason" => e,
                        "peer_id" => peer_id.peer_id_marker.clone(), "peer_ip" => peer_id.peer_address.to_string(), "peer" => peer_id.peer_ref.name(), "peer_uri" => peer_id.peer_ref.uri().to_string());
                    continue;
                }
            };

            // find next blocks
            let mut missing_blocks = Vec::with_capacity(available_queue_capacity);
            for branch in branches {
                if available_queue_capacity == 0 {
                    break;
                }
                branch.collect_next_blocks_to_download(
                    available_queue_capacity,
                    &already_queued,
                    &mut missing_blocks,
                );
                available_queue_capacity = available_queue_capacity
                    .checked_sub(missing_blocks.len())
                    .unwrap_or(0);
            }

            // try schedule
            if let Err(e) = requester.fetch_block_headers(missing_blocks, peer_id, peer_queues, log)
            {
                warn!(log, "Failed to schedule block headers for download from peer"; "reason" => e,
                        "peer_id" => peer_id.peer_id_marker.clone(), "peer_ip" => peer_id.peer_address.to_string(), "peer" => peer_id.peer_ref.name(), "peer_uri" => peer_id.peer_ref.uri().to_string());
            }
        }
    }

    pub fn schedule_operations_to_download(&mut self, requester: &DataRequester, log: &Logger) {
        for PeerBootstrapState {
            peer_id,
            peer_queues,
            branches,
            ..
        } in self.peers.values_mut()
        {
            // check peers blocks queue
            let (already_queued, mut available_queue_capacity) = match peer_queues
                .get_already_queued_block_operations_and_max_capacity()
            {
                Ok(queued_and_capacity) => queued_and_capacity,
                Err(e) => {
                    warn!(log, "Failed to get available operations queue capacity for peer, so ignore this run for peer"; "reason" => e,
                        "peer_id" => peer_id.peer_id_marker.clone(), "peer_ip" => peer_id.peer_address.to_string(), "peer" => peer_id.peer_ref.name(), "peer_uri" => peer_id.peer_ref.uri().to_string());
                    continue;
                }
            };

            // find next blocks
            let mut missing_blocks = Vec::with_capacity(available_queue_capacity);
            for branch in branches {
                if available_queue_capacity == 0 {
                    break;
                }
                branch.collect_next_block_operations_to_download(
                    available_queue_capacity,
                    &already_queued,
                    &mut missing_blocks,
                );
                available_queue_capacity = available_queue_capacity
                    .checked_sub(missing_blocks.len())
                    .unwrap_or(0);
            }

            // try schedule
            if let Err(e) =
                requester.fetch_block_operations(missing_blocks, peer_id, peer_queues, log)
            {
                warn!(log, "Failed to schedule block operations for download from peer"; "reason" => e,
                        "peer_id" => peer_id.peer_id_marker.clone(), "peer_ip" => peer_id.peer_address.to_string(), "peer" => peer_id.peer_ref.name(), "peer_uri" => peer_id.peer_ref.uri().to_string());
            }
        }
    }

    pub fn schedule_blocks_for_apply(
        &mut self,
        requester: &DataRequester,
        max_block_apply_batch: usize,
        chain_id: &Arc<ChainId>,
        peer_branch_bootstrapper: &PeerBranchBootstrapperRef,
    ) {
        let BootstrapState {
            peers,
            block_state_db,
        } = self;

        for PeerBootstrapState { branches, .. } in peers.values_mut() {
            for branch in branches {
                if let Some(batch) = branch.find_next_block_to_apply(max_block_apply_batch) {
                    // mark scheduled
                    block_state_db.mark_scheduled_for_apply(&batch);

                    branch.intervals.iter_mut().for_each(|i| {
                        i.blocks.iter_mut().for_each(|b| {
                            if batch.block_to_apply.eq(&b.block_hash)
                                || batch.successors.contains(&b.block_hash)
                            {
                                b.scheduled_for_apply = true;
                            }
                        })
                    });

                    // schedule
                    requester.call_schedule_apply_block(
                        chain_id.clone(),
                        batch,
                        Some(peer_branch_bootstrapper.clone()),
                    );
                }
            }
        }
    }

    pub fn block_downloaded(
        &mut self,
        block_hash: BlockHash,
        predecessor_block_hash: BlockHash,
        new_state: InnerBlockState,
    ) {
        let block_hash = Arc::new(block_hash);
        let predecessor_block_hash = Arc::new(predecessor_block_hash);

        // update pipelines
        self.peers.values_mut().for_each(|peer_state| {
            peer_state.branches.iter_mut().for_each(|branch| {
                branch.block_downloaded(&block_hash, &new_state, predecessor_block_hash.clone())
            });
        });
        // update db
        self.block_state_db
            .update_block(block_hash, predecessor_block_hash.clone(), new_state);
    }

    pub fn block_operations_downloaded(&mut self, block_hash: BlockHash) {
        // update
        self.peers.values_mut().for_each(|peer_state| {
            peer_state
                .branches
                .iter_mut()
                .for_each(|branch| branch.block_operations_downloaded(&block_hash));
        });

        // update db
        self.block_state_db
            .mark_block_operations_downloaded(&block_hash);
    }

    pub fn block_applied(&mut self, block_hash: &BlockHash) {
        // update
        self.peers.values_mut().for_each(|peer_state| {
            peer_state
                .branches
                .iter_mut()
                .for_each(|branch| branch.block_applied(block_hash));
        });
        // update db
        self.block_state_db.mark_block_applied(&block_hash);
    }

    pub fn check_bootstrapped_branches(&mut self, shell_channel: &ShellChannelRef, log: &Logger) {
        // remove all finished branches for every peer
        self.peers.values_mut().for_each(|PeerBootstrapState { branches, peer_id: peer, is_bootstrapped, empty_bootstrap_state, .. }| {
            branches
                .retain(|branch| {
                    if branch.is_done() {
                        info!(log, "Finished branch bootstrapping process";
                            "to_level" => &branch.to_level,
                            "peer_id" => peer.peer_id_marker.clone(), "peer_ip" => peer.peer_address.to_string(), "peer" => peer.peer_ref.name(), "peer_uri" => peer.peer_ref.uri().to_string());

                        // send for peer just once
                        if *is_bootstrapped == false {
                            *is_bootstrapped = true;
                            shell_channel.tell(
                                Publish {
                                    msg: ShellChannelMsg::PeerBranchSynchronizationDone(
                                        PeerBranchSynchronizationDone::new(peer.clone(), branch.to_level.clone()),
                                    ),
                                    topic: ShellChannelTopic::ShellCommands.into(),
                                },
                                None,
                            );
                        }

                        false
                    } else {
                        true
                    }
                });

            if branches.is_empty() && empty_bootstrap_state.is_none() {
                *empty_bootstrap_state = Some(Instant::now());
            }
        });
    }
}

/// BootstrapState helps to easily manage/mutate inner state
pub struct BranchState {
    /// Level of the highest block from all intervals
    to_level: Arc<Level>,

    /// Partitions are expected to be ordered from the lowest_level/oldest block
    intervals: Vec<BranchInterval>,
}

impl BranchState {
    /// Creates new pipeline, it must always start with applied block (at least with genesis),
    /// This block is used to define start of the branch
    pub fn new(
        first_applied_block: Arc<BlockHash>,
        blocks: Vec<Arc<BlockHash>>,
        to_level: Arc<Level>,
    ) -> BranchState {
        BranchState {
            intervals: BranchInterval::split(BlockState::new_applied(first_applied_block), blocks),
            to_level,
        }
    }

    /// Returns true if any interval contains requested block
    pub fn contains_block(&self, block: &BlockHash) -> bool {
        self.intervals.iter().any(|interval| {
            interval
                .blocks
                .iter()
                .any(|b| b.block_hash.as_ref().eq(block))
        })
    }

    /// Tries to merge/extends existing bootstrap (optimization)
    pub fn merge(
        &mut self,
        new_bootstrap_level: Arc<Level>,
        new_missing_history: &[Arc<BlockHash>],
    ) -> bool {
        // we can merge, only if the last block has continuation in new_bootstrap
        let mut new_intervals = Vec::new();

        if let Some(last_interval) = self.intervals.last() {
            if let Some(last_block) = last_interval.blocks.last() {
                // we need to find this last block in new_bootstrap
                if let Some(found_position) = new_missing_history
                    .iter()
                    .position(|b| b.as_ref().eq(last_block.block_hash.as_ref()))
                {
                    // check if we have more elements after found, means, we can add new interval
                    if (found_position + 1) < new_missing_history.len() {
                        // split to intervals
                        new_intervals.extend(BranchInterval::split(
                            last_block.clone(),
                            new_missing_history[(found_position + 1)..].to_vec(),
                        ));
                    }
                }
            }
        }

        if !new_intervals.is_empty() {
            self.intervals.extend(new_intervals);
            self.to_level = new_bootstrap_level;
            return true;
        }

        false
    }

    /// This finds block, which should be downloaded first and refreshes state for all touched blocks
    pub fn collect_next_blocks_to_download(
        &mut self,
        requested_count: usize,
        ignored_blocks: &HashSet<Arc<BlockHash>>,
        blocks_to_download: &mut Vec<Arc<BlockHash>>,
    ) {
        if requested_count == 0 {
            return;
        }

        // lets iterate intervals
        for interval in self.intervals.iter_mut() {
            // if interval is downloaded, just skip it
            if interval.all_blocks_downloaded {
                continue;
            }

            // let walk throught interval and resolve first missing block
            interval.collect_first_missing_block(ignored_blocks, blocks_to_download);

            // check if we have enought
            if blocks_to_download.len() >= requested_count {
                break;
            }
        }
    }

    /// This finds block, which we miss operations and should be downloaded first and also refreshes state for all touched blocks
    pub fn collect_next_block_operations_to_download(
        &mut self,
        requested_count: usize,
        ignored_blocks: &HashSet<Arc<BlockHash>>,
        blocks_to_download: &mut Vec<Arc<BlockHash>>,
    ) {
        if requested_count == 0 {
            return;
        }

        // lets iterate intervals
        for interval in self.intervals.iter_mut() {
            // if interval is downloaded, just skip it
            if interval.all_operations_downloaded {
                continue;
            }

            // let walk throught interval and resolve missing block from this interval
            interval.collect_block_with_missing_operations(
                requested_count,
                &ignored_blocks,
                blocks_to_download,
            );

            if blocks_to_download.len() >= requested_count {
                return;
            }
        }
    }

    /// This finds block, for which we should apply and also refreshes state for all touched blocks.
    ///
    /// IA - callback then returns if block is already aplpied
    pub fn find_next_block_to_apply(
        &mut self,
        max_block_apply_batch: usize,
    ) -> Option<ApplyBlockBatch> {
        let mut batch_for_apply: Option<ApplyBlockBatch> = None;

        let mut previous_block: Option<(Arc<BlockHash>, bool, bool)> = None;

        for interval in self.intervals.iter_mut() {
            if interval.all_block_applied {
                continue;
            }

            let mut interval_break = false;

            // check all blocks in interval
            // get first non-applied block
            for b in interval.blocks.iter_mut() {
                // skip applied or already scheduled
                if b.applied || b.scheduled_for_apply {
                    // continue to check next block
                    previous_block = Some((b.block_hash.clone(), b.applied, b.scheduled_for_apply));
                    continue;
                }

                // if previous is the same as a block - can happen on the border of interevals
                // where last block of previous interval is the first block of next interval
                if let Some((previous_block_hash, ..)) = previous_block.as_ref() {
                    if b.block_hash.as_ref().eq(previous_block_hash.as_ref()) {
                        // just continue, previous_block is the same and we processed it right before
                        continue;
                    }
                }

                // if block and operations are downloaded, we check his predecessor, if applied or scheduled for apply
                if b.block_downloaded && b.operations_downloaded {
                    // predecessor must match - continuos chain of blocks
                    if let Some(block_predecessor) = b.predecessor_block_hash.as_ref() {
                        if let Some((
                            previous_block_hash,
                            previous_is_applied,
                            previous_is_scheduled_for_apply,
                        )) = previous_block.as_ref()
                        {
                            if block_predecessor.as_ref().eq(previous_block_hash.as_ref()) {
                                // if we came here, we have still continuous chain

                                // if previos block is applied and b is not, then we have batch start candidate
                                if *previous_is_applied || *previous_is_scheduled_for_apply {
                                    // start batch and continue to next blocks
                                    batch_for_apply =
                                        Some(ApplyBlockBatch::start_batch(b.block_hash.clone()));

                                    if max_block_apply_batch > 0 {
                                        // continue to check next block
                                        previous_block = Some((
                                            b.block_hash.clone(),
                                            b.applied,
                                            b.scheduled_for_apply,
                                        ));
                                        continue;
                                    }
                                } else if let Some(batch) = batch_for_apply.as_mut() {
                                    // if previous block is not applied, means we can add it to batch
                                    batch.add_successor(b.block_hash.clone());
                                    if batch.successors_size() < max_block_apply_batch {
                                        // continue to check next block
                                        previous_block = Some((
                                            b.block_hash.clone(),
                                            b.applied,
                                            b.scheduled_for_apply,
                                        ));
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                }

                // if we did not find anything in this interval and this interval is not whole applied, there is no reason to continue to next interval
                interval_break = true;
                break;
            }

            // if we stop interval with break, we dont want to continue to next interval
            if interval_break {
                break;
            }
        }

        batch_for_apply
    }

    /// Notify state requested_block_hash was downloaded
    ///
    /// <BM> callback which return (bool as downloaded, bool as applied, bool as are_operations_complete)
    pub fn block_downloaded(
        &mut self,
        requested_block_hash: &BlockHash,
        requested_block_new_inner_state: &InnerBlockState,
        predecessor_block_hash: Arc<BlockHash>,
    ) {
        // find first interval with this block
        let interval_to_handle = self.intervals.iter_mut().enumerate().find(|(_, interval)| {
            // find first interval with this block
            interval
                .blocks
                .iter()
                .filter(|b| !b.applied)
                .any(|b| b.block_hash.as_ref().eq(requested_block_hash))
        });

        // Note:end_interval
        let mut end_of_interval: Option<(usize, InnerBlockState)> = None;

        if let Some((interaval_idx, interval)) = interval_to_handle {
            let block_count = interval.blocks.len();
            let mut predecessor_insert_to_index: Option<usize> = None;

            // update metadata
            for (block_index, b) in interval.blocks.iter_mut().enumerate() {
                // update request block as downloaded
                if requested_block_hash.eq(b.block_hash.as_ref()) {
                    // if applied, then skip
                    if b.applied {
                        continue;
                    }

                    // update its state
                    b.update(&requested_block_new_inner_state);
                    b.update_predecessor(predecessor_block_hash.clone());

                    predecessor_insert_to_index = Some(block_index);

                    // Note:end_interval: handle end of the interval
                    if (block_count - 1) == block_index {
                        end_of_interval = Some((
                            interaval_idx,
                            InnerBlockState {
                                block_downloaded: b.block_downloaded,
                                operations_downloaded: b.operations_downloaded,
                                applied: b.applied,
                            },
                        ));
                    }

                    break;
                }
            }

            if let Some(predecessor_insert_to_index) = predecessor_insert_to_index {
                // we cannot add predecessor before begining of interval
                if predecessor_insert_to_index > 0 {
                    // check if predecessor block is really our predecessor
                    if let Some(potential_predecessor) =
                        interval.blocks.get(predecessor_insert_to_index - 1)
                    {
                        // if not, we need to insert predecessor here
                        if !potential_predecessor
                            .block_hash
                            .as_ref()
                            .eq(&predecessor_block_hash)
                        {
                            interval.blocks.insert(
                                predecessor_insert_to_index,
                                BlockState::new(predecessor_block_hash),
                            );
                        }
                    }
                }
            }

            // check if have downloaded whole interval
            interval.check_all_blocks_downloaded();
        };

        // Note:end_interval we need to copy this to beginging of the next interval
        if let Some((interval_idx, end_of_previos_interval_state)) = end_of_interval {
            if let Some(next_interval) = self.intervals.get_mut(interval_idx + 1) {
                // get first block
                if let Some(begin) = next_interval.blocks.get_mut(0) {
                    begin.update(&end_of_previos_interval_state);
                }

                // check if have downloaded whole next interval
                next_interval.check_all_blocks_downloaded();
            }
        }

        // check, what if requested block is already applied (this removes all predecessor and/or the whole interval)
        if requested_block_new_inner_state.applied {
            self.block_applied(&requested_block_hash);
        }
    }

    /// Notify state requested_block_hash has all downloaded operations
    ///
    /// <BM> callback wchic return (bool as downloaded, bool as applied, bool as are_operations_complete)
    pub fn block_operations_downloaded(&mut self, requested_block_hash: &BlockHash) {
        // find first interval with this block
        let interval_to_handle = self.intervals.iter_mut().enumerate().find(|(_, interval)| {
            // find first interval with this block
            interval
                .blocks
                .iter()
                .filter(|b| !b.applied)
                .any(|b| b.block_hash.as_ref().eq(requested_block_hash))
        });

        // Note:end_interval
        let mut end_of_interval: Option<(usize, InnerBlockState)> = None;

        if let Some((interaval_idx, interval)) = interval_to_handle {
            // find block and update metadata
            let block_count = interval.blocks.len();
            for (block_index, mut b) in interval.blocks.iter_mut().enumerate() {
                // update request block as downloaded
                if requested_block_hash.eq(b.block_hash.as_ref()) {
                    // now we can update
                    if !b.operations_downloaded {
                        b.operations_downloaded = true;
                    }

                    // Note:end_interval: handle end of the interval
                    if (block_count - 1) == block_index {
                        end_of_interval = Some((
                            interaval_idx,
                            InnerBlockState {
                                block_downloaded: b.block_downloaded,
                                operations_downloaded: b.operations_downloaded,
                                applied: b.applied,
                            },
                        ));
                    }

                    break;
                }
            }

            interval.check_all_operations_downloaded();
        };

        // Note:end_interval we need to copy this to beginging of the next interval
        if let Some((interval_idx, end_of_previos_interval_state)) = end_of_interval {
            if let Some(next_interval) = self.intervals.get_mut(interval_idx + 1) {
                // get first block
                if let Some(begin) = next_interval.blocks.get_mut(0) {
                    begin.update(&end_of_previos_interval_state);
                }

                next_interval.check_all_operations_downloaded();
            }
        }
    }

    /// Notify state requested_block_hash has been applied
    ///
    /// <BM> callback wchic return (bool as downloaded, bool as applied, bool as are_operations_complete)
    pub fn block_applied(&mut self, requested_block_hash: &BlockHash) {
        // find first interval with this block
        let interval_to_handle = self.intervals.iter_mut().enumerate().find(|(_, interval)| {
            // find first interval with this block
            interval
                .blocks
                .iter()
                .any(|b| b.block_hash.as_ref().eq(requested_block_hash))
        });

        if let Some((interval_idx, interval)) = interval_to_handle {
            // Note:end_interval
            let mut end_of_interval: Option<(usize, InnerBlockState)> = None;

            // find block and update metadata
            let block_count = interval.blocks.len();
            let mut block_index_to_remove_before = None;
            for (block_index, mut b) in interval.blocks.iter_mut().enumerate() {
                // update request block as downloaded
                if requested_block_hash.eq(b.block_hash.as_ref()) {
                    // now we can update
                    if !b.applied {
                        b.applied = true;
                    }
                    if b.applied {
                        block_index_to_remove_before = Some(block_index);
                    }

                    // Note:end_interval: handle end of the interval
                    if (block_count - 1) == block_index {
                        end_of_interval = Some((
                            interval_idx,
                            InnerBlockState {
                                block_downloaded: b.block_downloaded,
                                operations_downloaded: b.operations_downloaded,
                                applied: b.applied,
                            },
                        ));
                    }

                    break;
                }
            }

            if let Some(remove_index) = block_index_to_remove_before {
                // we dont want to remove the block, just the blocks before, because at least first block must be applied
                for _ in 0..remove_index {
                    let _ = interval.blocks.remove(0);
                }
                interval.check_all_blocks_downloaded();
            }

            // Note:end_interval we need to copy this to beginging of the next interval
            if let Some((interval_idx, end_of_previos_interval_state)) = end_of_interval {
                if let Some(next_interval) = self.intervals.get_mut(interval_idx + 1) {
                    // get first block
                    if let Some(begin) = next_interval.blocks.get_mut(0) {
                        begin.update(&end_of_previos_interval_state);
                    }
                }
            }

            // remove interval if it is empty
            if let Some(interval_to_remove) = self.intervals.get_mut(interval_idx) {
                if interval_to_remove.blocks.len() <= 1 {
                    let all_applied = interval_to_remove.blocks.iter().all(|b| b.applied);
                    if all_applied {
                        let _ = self.intervals.remove(interval_idx);
                    }
                }

                // here we need to remove all previous interval, becuse we dont need them, when higher block was applied
                if interval_idx > 0 {
                    // so, remove all previous
                    for _ in 0..interval_idx {
                        let _ = self.intervals.remove(0);
                    }
                }
            }
        };
    }

    fn is_done(&self) -> bool {
        self.intervals.is_empty()
    }
}

/// Partions represents sequence from history, ordered from lowest_level/oldest block
///
/// History:
///     (bh1, bh2, bh3, bh4, bh5)
/// Is splitted to:
///     (bh1, bh2)
///     (bh2, bh3)
///     (bh3, bh4)
///     (bh4, bh5)
struct BranchInterval {
    all_blocks_downloaded: bool,
    all_operations_downloaded: bool,
    all_block_applied: bool,
    blocks: Vec<BlockStateRef>,
}

impl BranchInterval {
    fn new(left: BlockState) -> Self {
        Self {
            all_blocks_downloaded: false,
            all_operations_downloaded: false,
            all_block_applied: false,
            blocks: vec![left],
        }
    }

    fn new_with_left(left: BlockState, right_block_hash: Arc<BlockHash>) -> Self {
        Self {
            all_blocks_downloaded: false,
            all_operations_downloaded: false,
            all_block_applied: false,
            blocks: vec![left, BlockState::new(right_block_hash)],
        }
    }

    fn split(first_block_state: BlockState, blocks: Vec<Arc<BlockHash>>) -> Vec<BranchInterval> {
        let mut intervals: Vec<BranchInterval> = Vec::with_capacity(blocks.len() / 2);

        // insert first interval
        intervals.push(BranchInterval::new(first_block_state));

        // now split to interval the rest of the blocks
        for bh in blocks {
            let new_interval = match intervals.last_mut() {
                Some(last_part) => {
                    if last_part.blocks.len() >= 2 {
                        // previous is full, so add new one
                        if let Some(last_block) = last_part.blocks.last() {
                            Some(BranchInterval::new_with_left(last_block.clone(), bh))
                        } else {
                            // this cannot happen
                            last_part.blocks.push(BlockState::new(bh));
                            None
                        }
                    } else {
                        // close interval
                        last_part.blocks.push(BlockState::new(bh));
                        None
                    }
                }
                None => Some(BranchInterval::new(BlockState::new(bh))),
            };

            if let Some(new_interval) = new_interval {
                intervals.push(new_interval);
            }
        }

        intervals
    }

    /// Walks throught interval blocks and checks if we can feed it,
    fn collect_first_missing_block(
        &mut self,
        ignored_blocks: &HashSet<Arc<BlockHash>>,
        blocks_to_download: &mut Vec<Arc<BlockHash>>,
    ) {
        let mut stop_block: Option<Arc<BlockHash>> = None;
        let mut start_from_the_begining = true;
        while start_from_the_begining {
            start_from_the_begining = false;

            let mut insert_predecessor: Option<(usize, Arc<BlockHash>)> = None;
            let mut previous: Option<Arc<BlockHash>> = None;

            for (idx, b) in self.blocks.iter_mut().enumerate() {
                // stop block optimization, because we are just prepending to blocks (optimization)
                if let Some(stop_block) = stop_block.as_ref() {
                    if stop_block.eq(&b.block_hash) {
                        break;
                    }
                }

                if !b.block_downloaded {
                    if ignored_blocks.contains(&b.block_hash) {
                        // already scheduled
                        return;
                    }
                    if !blocks_to_download.contains(&b.block_hash) {
                        // add for download
                        blocks_to_download.push(b.block_hash.clone());
                    }
                    return;
                }

                // check if we missing predecessor in the interval (skipping check for the first block)
                if let Some(previous_block) = previous.as_ref() {
                    match b.predecessor_block_hash.as_ref() {
                        Some(block_predecessor) => {
                            if !previous_block.as_ref().eq(&block_predecessor) {
                                // if previos block is not our predecessor, we need to insert it there
                                insert_predecessor = Some((idx, block_predecessor.clone()));
                                stop_block = Some(b.block_hash.clone());
                                break;
                            }
                        }
                        None => {
                            // strange, we dont know predecessor of block, so we schedule block downloading once more
                            if ignored_blocks.contains(&b.block_hash) {
                                // already scheduled
                                return;
                            }
                            if !blocks_to_download.contains(&b.block_hash) {
                                // add for download
                                blocks_to_download.push(b.block_hash.clone());
                            }
                            return;
                        }
                    }
                }

                previous = Some(b.block_hash.clone());
            }

            // handle missing predecessor
            if let Some((predecessor_idx, predecessor_block_hash)) = insert_predecessor {
                // create actual state for predecessor
                let predecessor_state = BlockState::new(predecessor_block_hash);

                // insert predecessor (we cannot insert predecessor before beginging of interval)
                if predecessor_idx != 0 {
                    self.blocks.insert(predecessor_idx, predecessor_state);
                }
                start_from_the_begining = true;
            }
        }

        self.check_all_blocks_downloaded();
    }

    /// Check, if we had downloaded the whole interval,
    /// if flag ['all_blocks_downloaded'] was set, then returns true.
    fn check_all_blocks_downloaded(&mut self) {
        if self.all_blocks_downloaded {
            return;
        }

        // if we came here, we just need to check if we downloaded the whole interval
        let mut previous: Option<&Arc<BlockHash>> = None;
        let mut all_blocks_downloaded = true;

        for b in &self.blocks {
            if !b.block_downloaded {
                all_blocks_downloaded = false;
                break;
            }
            if let Some(previous) = previous {
                if let Some(block_predecessor) = &b.predecessor_block_hash {
                    if !block_predecessor.eq(&previous) {
                        all_blocks_downloaded = false;
                        break;
                    }
                } else {
                    // missing predecessor
                    all_blocks_downloaded = false;
                    break;
                }
            }
            previous = Some(&b.block_hash);
        }

        if all_blocks_downloaded {
            self.all_blocks_downloaded = true;
            self.check_all_operations_downloaded();
        }
    }

    fn check_all_operations_downloaded(&mut self) {
        // check interval has all operations downloaded
        if self.all_blocks_downloaded {
            self.all_operations_downloaded = self.blocks.iter().all(|b| b.operations_downloaded);
        }
    }

    pub(crate) fn collect_block_with_missing_operations(
        &mut self,
        requested_count: usize,
        ignored_blocks: &HashSet<Arc<BlockHash>>,
        blocks_to_download: &mut Vec<Arc<BlockHash>>,
    ) {
        if self.all_operations_downloaded {
            return;
        }

        for b in self.blocks.iter_mut() {
            // if downloaded, just skip
            if b.operations_downloaded {
                continue;
            }

            // skip already scheduled
            if ignored_blocks.contains(&b.block_hash) || blocks_to_download.contains(&b.block_hash)
            {
                continue;
            }

            // check result
            if blocks_to_download.len() >= requested_count {
                break;
            }

            // schedule for download
            blocks_to_download.push(b.block_hash.clone());
        }
    }
}

#[derive(Clone, Debug)]
pub struct InnerBlockState {
    pub block_downloaded: bool,
    pub operations_downloaded: bool,
    pub applied: bool,
}

#[derive(Clone)]
struct BlockState {
    block_hash: Arc<BlockHash>,
    predecessor_block_hash: Option<Arc<BlockHash>>,

    block_downloaded: bool,
    operations_downloaded: bool,
    applied: bool,
    scheduled_for_apply: bool,
}

impl BlockState {
    fn new(block_hash: Arc<BlockHash>) -> Self {
        BlockState {
            block_hash,
            predecessor_block_hash: None,
            block_downloaded: false,
            operations_downloaded: false,
            applied: false,
            scheduled_for_apply: false,
        }
    }

    fn new_applied(block_hash: Arc<BlockHash>) -> Self {
        BlockState {
            block_hash,
            predecessor_block_hash: None,
            block_downloaded: true,
            operations_downloaded: true,
            applied: true,
            scheduled_for_apply: false,
        }
    }

    fn update(&mut self, new_state: &InnerBlockState) {
        if new_state.block_downloaded {
            if !self.block_downloaded {
                self.block_downloaded = true;
            }
        }

        if new_state.applied {
            if !self.applied {
                self.applied = true;
            }
        }

        if new_state.operations_downloaded {
            if !self.operations_downloaded {
                self.operations_downloaded = true;
            }
        }
    }

    fn update_predecessor(&mut self, predecessor: Arc<BlockHash>) {
        if self.predecessor_block_hash.is_none() {
            self.predecessor_block_hash = Some(predecessor);
        }
    }
}

/// Internal state for peer
pub struct PeerBootstrapState {
    /// Peer's identification
    peer_id: Arc<PeerId>,
    /// Peer's shared queues
    peer_queues: Arc<DataQueues>,

    /// List of branches from peer
    branches: Vec<BranchState>,

    /// if peer was bootstrapped
    is_bootstrapped: bool,

    /// Indicates stalled peer, after all branches were resolved and cleared, how long we did not receive new branch
    empty_bootstrap_state: Option<Instant>,
}

pub struct BlockStateDb {
    blocks: HashMap<Arc<BlockHash>, BlockStateRef>,
}

impl BlockStateDb {
    fn new(initial_capacity: usize) -> Self {
        Self {
            blocks: HashMap::with_capacity(initial_capacity),
        }
    }

    pub fn update_block(
        &mut self,
        block_hash: Arc<BlockHash>,
        predecessor_block_hash: Arc<BlockHash>,
        new_state: InnerBlockState,
    ) {
        let state = self
            .blocks
            .entry(block_hash.clone())
            .or_insert_with(|| BlockStateRef::new(block_hash));
        state.update(&new_state);
        state.update_predecessor(predecessor_block_hash);
    }

    pub fn mark_scheduled_for_apply(&mut self, batch: &ApplyBlockBatch) {
        // mark batch start
        if let Some(block_state) = self.blocks.get_mut(&batch.block_to_apply) {
            block_state.scheduled_for_apply = true;
        }

        // mark batch successors
        batch.successors.iter().for_each(|b| {
            if let Some(block_state) = self.blocks.get_mut(b.as_ref()) {
                block_state.scheduled_for_apply = true;
            }
        })
    }

    pub fn mark_block_operations_downloaded(&mut self, block_hash: &BlockHash) {
        // mark batch start
        if let Some(block_state) = self.blocks.get_mut(block_hash) {
            block_state.operations_downloaded = true;
        }
    }

    pub fn mark_block_applied(&mut self, block_hash: &BlockHash) {
        // TODO: all predecessor mark as applied and remove from memory?
        // mark batch start
        // let mut block_state = None;
        if let Some(block_state) = self.blocks.get_mut(block_hash) {
            block_state.applied = true;
            block_state.scheduled_for_apply = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use serial_test::serial;

    use networking::p2p::network_channel::NetworkChannel;

    use crate::state::peer_state::DataQueuesLimits;
    use crate::state::tests::block;
    use crate::state::tests::prerequisites::{
        create_logger, create_test_actor_system, create_test_tokio_runtime, test_peer,
    };
    use crate::state::StateError;

    use super::*;

    macro_rules! hash_set {
        ( $( $x:expr ),* ) => {
            {
                let mut temp_set = HashSet::new();
                $(
                    temp_set.insert($x);
                )*
                temp_set
            }
        };
    }

    #[test]
    #[serial]
    fn test_bootstrap_state_add_new_branch() {
        // actors stuff
        let log = create_logger(slog::Level::Info);
        let sys = create_test_actor_system(log.clone());
        let runtime = create_test_tokio_runtime();
        let network_channel =
            NetworkChannel::actor(&sys).expect("Failed to create network channel");

        // peer1
        let peer_id = test_peer(&sys, network_channel, &runtime, 1234).peer_id;
        let peer_queues = Arc::new(DataQueues::new(DataQueuesLimits {
            max_queued_block_headers_count: 10,
            max_queued_block_operations_count: 10,
        }));

        // empty state
        let mut state = BootstrapState::default();

        // genesis
        let last_applied = block(0);

        // history blocks
        let history: Vec<Arc<BlockHash>> = vec![
            block(2),
            block(5),
            block(8),
            block(10),
            block(13),
            block(15),
            block(20),
        ];

        // add new branch to empty state
        state.add_new_branch(
            peer_id.clone(),
            peer_queues.clone(),
            last_applied.clone(),
            history,
            Arc::new(20),
            5,
            &log,
        );

        // check state
        let peer_bootstrap_state = state.peers.get_mut(peer_id.peer_ref.uri()).unwrap();
        assert!(peer_bootstrap_state.empty_bootstrap_state.is_none());
        assert_eq!(1, peer_bootstrap_state.branches.len());
        let pipeline = &peer_bootstrap_state.branches[0];
        assert_eq!(pipeline.intervals.len(), 7);
        assert_interval(&pipeline.intervals[0], (block(0), block(2)));
        assert_interval(&pipeline.intervals[1], (block(2), block(5)));
        assert_interval(&pipeline.intervals[2], (block(5), block(8)));
        assert_interval(&pipeline.intervals[3], (block(8), block(10)));
        assert_interval(&pipeline.intervals[4], (block(10), block(13)));
        assert_interval(&pipeline.intervals[5], (block(13), block(15)));
        assert_interval(&pipeline.intervals[6], (block(15), block(20)));

        // try add new branch merge new - ok
        let history_to_merge: Vec<Arc<BlockHash>> = vec![
            block(13),
            block(15),
            block(20),
            block(22),
            block(23),
            block(25),
            block(29),
        ];
        state.add_new_branch(
            peer_id.clone(),
            peer_queues.clone(),
            last_applied.clone(),
            history_to_merge,
            Arc::new(29),
            5,
            &log,
        );

        // check state - branch was extended
        let peer_bootstrap_state = state.peers.get_mut(peer_id.peer_ref.uri()).unwrap();
        assert!(peer_bootstrap_state.empty_bootstrap_state.is_none());
        assert_eq!(1, peer_bootstrap_state.branches.len());

        let pipeline = &peer_bootstrap_state.branches[0];
        assert_eq!(pipeline.intervals.len(), 11);
        assert_interval(&pipeline.intervals[0], (block(0), block(2)));
        assert_interval(&pipeline.intervals[1], (block(2), block(5)));
        assert_interval(&pipeline.intervals[2], (block(5), block(8)));
        assert_interval(&pipeline.intervals[3], (block(8), block(10)));
        assert_interval(&pipeline.intervals[4], (block(10), block(13)));
        assert_interval(&pipeline.intervals[5], (block(13), block(15)));
        assert_interval(&pipeline.intervals[6], (block(15), block(20)));
        assert_interval(&pipeline.intervals[7], (block(20), block(22)));
        assert_interval(&pipeline.intervals[8], (block(22), block(23)));
        assert_interval(&pipeline.intervals[9], (block(23), block(25)));
        assert_interval(&pipeline.intervals[10], (block(25), block(29)));

        // add next branch
        let history_to_merge: Vec<Arc<BlockHash>> = vec![
            block(113),
            block(115),
            block(120),
            block(122),
            block(123),
            block(125),
            block(129),
        ];
        state.add_new_branch(
            peer_id.clone(),
            peer_queues.clone(),
            last_applied.clone(),
            history_to_merge,
            Arc::new(129),
            5,
            &log,
        );

        // check state - branch was extended
        let peer_bootstrap_state = state.peers.get_mut(peer_id.peer_ref.uri()).unwrap();
        assert!(peer_bootstrap_state.empty_bootstrap_state.is_none());
        assert_eq!(2, peer_bootstrap_state.branches.len());

        // try add next branch - max branches 2 - no added
        let history_to_merge: Vec<Arc<BlockHash>> = vec![
            block(213),
            block(215),
            block(220),
            block(222),
            block(223),
            block(225),
            block(229),
        ];
        state.add_new_branch(
            peer_id.clone(),
            peer_queues,
            last_applied,
            history_to_merge,
            Arc::new(229),
            2,
            &log,
        );

        // check state - branch was extended - no
        let peer_bootstrap_state = state.peers.get_mut(peer_id.peer_ref.uri()).unwrap();
        assert!(peer_bootstrap_state.empty_bootstrap_state.is_none());
        assert_eq!(2, peer_bootstrap_state.branches.len());
    }

    #[test]
    fn test_bootstrap_state_split_to_intervals() {
        // genesis
        let last_applied = block(0);
        // history blocks
        let history: Vec<Arc<BlockHash>> = vec![
            block(2),
            block(5),
            block(8),
            block(10),
            block(13),
            block(15),
            block(20),
        ];

        // create
        let pipeline = BranchState::new(last_applied.clone(), history, Arc::new(20));
        assert_eq!(pipeline.intervals.len(), 7);
        assert_interval(&pipeline.intervals[0], (block(0), block(2)));
        assert_interval(&pipeline.intervals[1], (block(2), block(5)));
        assert_interval(&pipeline.intervals[2], (block(5), block(8)));
        assert_interval(&pipeline.intervals[3], (block(8), block(10)));
        assert_interval(&pipeline.intervals[4], (block(10), block(13)));
        assert_interval(&pipeline.intervals[5], (block(13), block(15)));
        assert_interval(&pipeline.intervals[6], (block(15), block(20)));

        assert!(pipeline.contains_block(&block(0)));
        assert!(pipeline.contains_block(&block(2)));
        assert!(pipeline.contains_block(&block(5)));
        assert!(pipeline.contains_block(&block(10)));
        assert!(pipeline.contains_block(&block(13)));
        assert!(pipeline.contains_block(&block(15)));
        assert!(pipeline.contains_block(&block(20)));
        assert!(!pipeline.contains_block(&block(1)));
        assert!(!pipeline.contains_block(&block(3)));
        assert!(!pipeline.contains_block(&block(4)));

        // check applied is just first block
        pipeline
            .intervals
            .iter()
            .map(|i| &i.blocks)
            .flatten()
            .for_each(|b| {
                if b.block_hash.as_ref().eq(last_applied.as_ref()) {
                    assert!(b.applied);
                    assert!(b.block_downloaded);
                    assert!(b.operations_downloaded);
                } else {
                    assert!(!b.applied);
                    assert!(!b.block_downloaded);
                    assert!(!b.operations_downloaded);
                }
            })
    }

    #[test]
    fn test_bootstrap_state_merge() {
        // genesis
        let last_applied = block(0);

        // history blocks
        let history1: Vec<Arc<BlockHash>> = vec![
            block(2),
            block(5),
            block(8),
            block(10),
            block(13),
            block(15),
            block(20),
        ];
        let mut pipeline1 = BranchState::new(last_applied.clone(), history1, Arc::new(20));
        assert_eq!(pipeline1.intervals.len(), 7);
        assert_eq!(*pipeline1.to_level, 20);

        // try merge lower - nothing
        let history_to_merge: Vec<Arc<BlockHash>> = vec![
            block(2),
            block(5),
            block(8),
            block(10),
            block(13),
            block(15),
        ];
        assert!(!pipeline1.merge(Arc::new(100), &history_to_merge));
        assert_eq!(pipeline1.intervals.len(), 7);
        assert_eq!(*pipeline1.to_level, 20);

        // try merge different - nothing
        let history_to_merge: Vec<Arc<BlockHash>> = vec![
            block(1),
            block(4),
            block(7),
            block(9),
            block(12),
            block(14),
            block(16),
        ];
        assert!(!pipeline1.merge(Arc::new(100), &history_to_merge));
        assert_eq!(pipeline1.intervals.len(), 7);
        assert_eq!(*pipeline1.to_level, 20);

        // try merge the same - nothing
        let history_to_merge: Vec<Arc<BlockHash>> = vec![
            block(2),
            block(5),
            block(8),
            block(10),
            block(13),
            block(15),
            block(20),
        ];
        assert!(!pipeline1.merge(Arc::new(100), &history_to_merge));
        assert_eq!(pipeline1.intervals.len(), 7);
        assert_eq!(*pipeline1.to_level, 20);

        // try merge the one new - nothing
        let history_to_merge: Vec<Arc<BlockHash>> = vec![
            block(2),
            block(5),
            block(8),
            block(10),
            block(13),
            block(15),
            block(20),
            block(22),
        ];
        assert!(pipeline1.merge(Arc::new(22), &history_to_merge));
        assert_eq!(pipeline1.intervals.len(), 8);
        assert_eq!(*pipeline1.to_level, 22);

        // try merge new - ok
        let history_to_merge: Vec<Arc<BlockHash>> = vec![
            block(2),
            block(5),
            block(8),
            block(10),
            block(13),
            block(15),
            block(20),
            block(22),
            block(23),
            block(25),
            block(29),
        ];
        assert!(pipeline1.merge(Arc::new(29), &history_to_merge));
        assert_eq!(pipeline1.intervals.len(), 11);
        assert_eq!(*pipeline1.to_level, 29);

        assert_interval(&pipeline1.intervals[0], (block(0), block(2)));
        assert_interval(&pipeline1.intervals[1], (block(2), block(5)));
        assert_interval(&pipeline1.intervals[2], (block(5), block(8)));
        assert_interval(&pipeline1.intervals[3], (block(8), block(10)));
        assert_interval(&pipeline1.intervals[4], (block(10), block(13)));
        assert_interval(&pipeline1.intervals[5], (block(13), block(15)));
        assert_interval(&pipeline1.intervals[6], (block(15), block(20)));
        assert_interval(&pipeline1.intervals[7], (block(20), block(22)));
        assert_interval(&pipeline1.intervals[8], (block(22), block(23)));
        assert_interval(&pipeline1.intervals[9], (block(23), block(25)));
        assert_interval(&pipeline1.intervals[10], (block(25), block(29)));
    }

    #[test]
    fn test_bootstrap_state_block_downloaded() -> Result<(), StateError> {
        // genesis
        let last_applied = block(0);
        // history blocks
        let history: Vec<Arc<BlockHash>> = vec![
            block(2),
            block(5),
            block(8),
            block(10),
            block(13),
            block(15),
            block(20),
        ];

        // create
        let mut pipeline = BranchState::new(last_applied, history, Arc::new(20));
        assert_eq!(pipeline.intervals.len(), 7);
        assert_interval(&pipeline.intervals[0], (block(0), block(2)));
        assert_interval(&pipeline.intervals[1], (block(2), block(5)));
        assert_interval(&pipeline.intervals[2], (block(5), block(8)));
        assert_interval(&pipeline.intervals[3], (block(8), block(10)));
        assert_interval(&pipeline.intervals[4], (block(10), block(13)));
        assert_interval(&pipeline.intervals[5], (block(13), block(15)));
        assert_interval(&pipeline.intervals[6], (block(15), block(20)));

        // register downloaded block 2 with his predecessor 1
        assert!(!pipeline.intervals[0].all_blocks_downloaded);
        assert_eq!(pipeline.intervals[0].blocks.len(), 2);
        let mut result = Vec::new();
        pipeline.collect_next_blocks_to_download(1, &HashSet::default(), &mut result);
        assert_eq!(1, result.len());
        assert_eq!(result[0].as_ref(), block(2).as_ref());

        pipeline.block_downloaded(
            &block(2),
            &InnerBlockState {
                block_downloaded: true,
                applied: false,
                operations_downloaded: false,
            },
            block(1),
        );
        // interval not closed
        assert!(!pipeline.intervals[0].all_blocks_downloaded);
        assert_eq!(pipeline.intervals[0].blocks.len(), 3);

        let mut result = Vec::new();
        pipeline.collect_next_blocks_to_download(1, &HashSet::default(), &mut result);
        assert_eq!(1, result.len());
        assert_eq!(result[0].as_ref(), block(1).as_ref());

        // register downloaded block 1
        pipeline.block_downloaded(
            &block(1),
            &InnerBlockState {
                block_downloaded: true,
                applied: false,
                operations_downloaded: false,
            },
            block(0),
        );
        // interval is closed
        assert!(pipeline.intervals[0].all_blocks_downloaded);
        assert_eq!(pipeline.intervals[0].blocks.len(), 3);

        // next interval
        assert!(!pipeline.intervals[1].all_blocks_downloaded);
        assert_eq!(pipeline.intervals[1].blocks.len(), 2);

        let mut result = Vec::new();
        pipeline.collect_next_blocks_to_download(1, &HashSet::default(), &mut result);
        assert_eq!(1, result.len());
        assert_eq!(result[0].as_ref(), block(5).as_ref());

        // register downloaded block 5 with his predecessor 4
        pipeline.block_downloaded(
            &block(5),
            &InnerBlockState {
                block_downloaded: true,
                applied: false,
                operations_downloaded: false,
            },
            block(4),
        );
        assert!(!pipeline.intervals[1].all_blocks_downloaded);
        assert_eq!(pipeline.intervals[1].blocks.len(), 3);

        // register downloaded block 4 as applied
        pipeline.block_downloaded(
            &block(4),
            &InnerBlockState {
                block_downloaded: true,
                applied: true,
                operations_downloaded: false,
            },
            block(3),
        );

        // first interval with block 2 and block 3 were removed, because if 4 is applied, 2/3 must be also
        assert!(pipeline.intervals[0].all_blocks_downloaded);
        assert_eq!(pipeline.intervals[0].blocks.len(), 2);
        assert_eq!(
            pipeline.intervals[0].blocks[0].block_hash.as_ref(),
            block(4).as_ref()
        );
        assert_eq!(
            pipeline.intervals[0].blocks[1].block_hash.as_ref(),
            block(5).as_ref()
        );

        assert!(!pipeline.intervals[1].all_blocks_downloaded);
        assert_eq!(pipeline.intervals[1].blocks.len(), 2);
        assert_eq!(
            pipeline.intervals[1].blocks[0].block_hash.as_ref(),
            block(5).as_ref()
        );
        assert_eq!(
            pipeline.intervals[1].blocks[1].block_hash.as_ref(),
            block(8).as_ref()
        );

        Ok(())
    }

    #[test]
    fn test_bootstrap_state_block_applied() {
        // genesis
        let last_applied = block(0);
        // history blocks
        let history: Vec<Arc<BlockHash>> = vec![
            block(2),
            block(5),
            block(8),
            block(10),
            block(13),
            block(15),
            block(20),
        ];

        // create
        let mut pipeline = BranchState::new(last_applied, history, Arc::new(20));
        assert_eq!(pipeline.intervals.len(), 7);
        assert_interval(&pipeline.intervals[0], (block(0), block(2)));
        assert_interval(&pipeline.intervals[1], (block(2), block(5)));
        assert_interval(&pipeline.intervals[2], (block(5), block(8)));
        assert_interval(&pipeline.intervals[3], (block(8), block(10)));
        assert_interval(&pipeline.intervals[4], (block(10), block(13)));
        assert_interval(&pipeline.intervals[5], (block(13), block(15)));
        assert_interval(&pipeline.intervals[6], (block(15), block(20)));

        // check intervals
        assert_eq!(pipeline.intervals.len(), 7);

        // check first block (block(2) ) from next 1 interval
        assert!(!pipeline.intervals[1].blocks[0].applied);
        assert!(!pipeline.intervals[1].blocks[0].block_downloaded);
        assert!(!pipeline.intervals[1].blocks[0].operations_downloaded);

        // register downloaded block 2 which is applied
        pipeline.block_downloaded(
            &block(2),
            &InnerBlockState {
                block_downloaded: true,
                applied: true,
                operations_downloaded: false,
            },
            block(1),
        );
        // interval 0 was removed
        assert_eq!(pipeline.intervals.len(), 6);
        // and first block of next interval is marked the same as the last block from 0 inerval, becauase it is the same block (block(2))
        // check first block (block(2) ) from next 1 interval
        assert!(pipeline.intervals[0].blocks[0].applied);
        assert!(pipeline.intervals[0].blocks[0].block_downloaded);
        assert!(!pipeline.intervals[0].blocks[0].operations_downloaded);
    }

    #[test]
    fn test_bootstrap_state_block_applied_marking() {
        // genesis
        let last_applied = block(0);
        // history blocks
        let history: Vec<Arc<BlockHash>> = vec![
            block(2),
            block(5),
            block(8),
            block(10),
            block(13),
            block(15),
            block(20),
        ];

        // create
        let mut pipeline = BranchState::new(last_applied, history, Arc::new(20));
        assert_eq!(pipeline.intervals.len(), 7);
        assert_interval(&pipeline.intervals[0], (block(0), block(2)));
        assert_interval(&pipeline.intervals[1], (block(2), block(5)));
        assert_interval(&pipeline.intervals[2], (block(5), block(8)));
        assert_interval(&pipeline.intervals[3], (block(8), block(10)));
        assert_interval(&pipeline.intervals[4], (block(10), block(13)));
        assert_interval(&pipeline.intervals[5], (block(13), block(15)));
        assert_interval(&pipeline.intervals[6], (block(15), block(20)));

        // check intervals
        assert_eq!(pipeline.intervals.len(), 7);

        // trigger that block 2 is download with predecessor 1
        pipeline.block_downloaded(
            &block(2),
            &InnerBlockState {
                block_downloaded: true,
                applied: false,
                operations_downloaded: false,
            },
            block(1),
        );
        // mark 1 as applied (half of interval)
        pipeline.block_applied(&block(1));
        assert_eq!(pipeline.intervals.len(), 7);
        // begining of interval is changed to block1
        assert_eq!(pipeline.intervals[0].blocks.len(), 2);
        assert_eq!(
            pipeline.intervals[0].blocks[0].block_hash.as_ref(),
            block(1).as_ref()
        );
        assert_eq!(
            pipeline.intervals[0].blocks[1].block_hash.as_ref(),
            block(2).as_ref()
        );

        // trigger that block 2 is applied
        pipeline.block_applied(&block(2));
        assert_eq!(pipeline.intervals.len(), 6);

        // trigger that block 8 is applied
        pipeline.block_applied(&block(8));
        for (id, i) in pipeline.intervals.iter().enumerate() {
            println!(
                "{} : {:?}",
                id,
                i.blocks
                    .iter()
                    .map(|b| b.block_hash.as_ref().clone())
                    .collect::<Vec<_>>()
            );
        }
        assert_eq!(pipeline.intervals.len(), 4);

        // trigger that last block is applied
        pipeline.block_applied(&block(20));

        println!("inevals: {}", pipeline.intervals.len());

        // interval 0 was removed
        assert!(pipeline.is_done());
    }

    #[test]
    fn test_collect_next_blocks_to_download() {
        // genesis
        let last_applied = block(0);
        // history blocks
        let history: Vec<Arc<BlockHash>> = vec![
            block(2),
            block(5),
            block(8),
            block(10),
            block(13),
            block(15),
            block(20),
        ];

        // create
        let mut pipeline = BranchState::new(last_applied, history, Arc::new(20));
        assert_eq!(pipeline.intervals.len(), 7);
        assert_interval(&pipeline.intervals[0], (block(0), block(2)));
        assert_interval(&pipeline.intervals[1], (block(2), block(5)));
        assert_interval(&pipeline.intervals[2], (block(5), block(8)));
        assert_interval(&pipeline.intervals[3], (block(8), block(10)));
        assert_interval(&pipeline.intervals[4], (block(10), block(13)));
        assert_interval(&pipeline.intervals[5], (block(13), block(15)));
        assert_interval(&pipeline.intervals[6], (block(15), block(20)));

        // check 2. inerval -  is not downloaded
        assert!(!pipeline.intervals[1].all_blocks_downloaded);
        assert_eq!(2, pipeline.intervals[1].blocks.len());

        // try to get blocks for download - max 5
        let mut blocks_to_download = Vec::new();
        pipeline.collect_next_blocks_to_download(5, &HashSet::default(), &mut blocks_to_download);
        assert_eq!(5, blocks_to_download.len());
        assert_eq!(blocks_to_download[0].as_ref(), block(2).as_ref());
        assert_eq!(blocks_to_download[1].as_ref(), block(5).as_ref());
        assert_eq!(blocks_to_download[2].as_ref(), block(8).as_ref());
        assert_eq!(blocks_to_download[3].as_ref(), block(10).as_ref());
        assert_eq!(blocks_to_download[4].as_ref(), block(13).as_ref());

        // try to get blocks for download - max 2
        let mut blocks_to_download = Vec::new();
        pipeline.collect_next_blocks_to_download(2, &HashSet::default(), &mut blocks_to_download);
        assert_eq!(2, blocks_to_download.len());
        assert_eq!(blocks_to_download[0].as_ref(), block(2).as_ref());
        assert_eq!(blocks_to_download[1].as_ref(), block(5).as_ref());

        // try to get blocks for download - max 2 interval with ignored
        let mut blocks_to_download = Vec::new();
        pipeline.collect_next_blocks_to_download(2, &hash_set![block(5)], &mut blocks_to_download);
        assert_eq!(2, blocks_to_download.len());
        assert_eq!(blocks_to_download[0].as_ref(), block(2).as_ref());
        assert_eq!(blocks_to_download[1].as_ref(), block(8).as_ref());
    }

    #[test]
    fn test_collect_block_with_missing_operations() -> Result<(), StateError> {
        // genesis
        let last_applied = block(0);
        // history blocks
        let history: Vec<Arc<BlockHash>> = vec![
            block(2),
            block(5),
            block(8),
            block(10),
            block(13),
            block(15),
            block(20),
        ];

        // create
        let mut pipeline = BranchState::new(last_applied, history, Arc::new(20));
        assert_eq!(pipeline.intervals.len(), 7);
        assert_interval(&pipeline.intervals[0], (block(0), block(2)));
        assert_interval(&pipeline.intervals[1], (block(2), block(5)));
        assert_interval(&pipeline.intervals[2], (block(5), block(8)));
        assert_interval(&pipeline.intervals[3], (block(8), block(10)));
        assert_interval(&pipeline.intervals[4], (block(10), block(13)));
        assert_interval(&pipeline.intervals[5], (block(13), block(15)));
        assert_interval(&pipeline.intervals[6], (block(15), block(20)));

        // check 1. inerval -  is not downloaded
        assert!(!pipeline.intervals[0].all_operations_downloaded);

        // download block 2 (but operations still misssing)
        pipeline.block_downloaded(
            &block(2),
            &InnerBlockState {
                block_downloaded: true,
                applied: false,
                operations_downloaded: false,
            },
            block(1),
        );

        // get next for download (meanwhile operations for 2 was downloaded)
        let mut missing_blocks = Vec::new();
        pipeline.intervals[0].collect_block_with_missing_operations(
            2,
            &HashSet::default(),
            &mut missing_blocks,
        );
        assert_eq!(2, missing_blocks.len());
        assert_eq!(missing_blocks[0], block(1));
        assert_eq!(missing_blocks[1], block(2));
        assert!(!pipeline.intervals[0].all_operations_downloaded);

        // get next for download with ignored blocks
        let mut missing_blocks = Vec::new();
        pipeline.intervals[1].collect_block_with_missing_operations(
            2,
            &hash_set![block(2)],
            &mut missing_blocks,
        );
        assert_eq!(1, missing_blocks.len());
        assert_eq!(missing_blocks[0], block(5));
        assert!(!pipeline.intervals[0].all_operations_downloaded);

        // get next for download with ignored blocks
        let mut missing_blocks = Vec::new();
        pipeline.intervals[2].collect_block_with_missing_operations(
            2,
            &hash_set![block(1)],
            &mut missing_blocks,
        );
        assert_eq!(2, missing_blocks.len());
        assert_eq!(missing_blocks[0], block(5));
        assert_eq!(missing_blocks[1], block(8));
        assert!(!pipeline.intervals[0].all_operations_downloaded);

        Ok(())
    }

    #[test]
    fn test_collect_next_block_operations_to_download() {
        // genesis
        let last_applied = block(0);
        // history blocks
        let history: Vec<Arc<BlockHash>> = vec![
            block(2),
            block(5),
            block(8),
            block(10),
            block(13),
            block(15),
            block(20),
        ];

        // create
        let mut pipeline = BranchState::new(last_applied, history, Arc::new(20));
        assert_eq!(pipeline.intervals.len(), 7);
        assert_interval(&pipeline.intervals[0], (block(0), block(2)));
        assert_interval(&pipeline.intervals[1], (block(2), block(5)));
        assert_interval(&pipeline.intervals[2], (block(5), block(8)));
        assert_interval(&pipeline.intervals[3], (block(8), block(10)));
        assert_interval(&pipeline.intervals[4], (block(10), block(13)));
        assert_interval(&pipeline.intervals[5], (block(13), block(15)));
        assert_interval(&pipeline.intervals[6], (block(15), block(20)));

        // check 2. inerval -  is not downloaded
        assert!(!pipeline.intervals[1].all_blocks_downloaded);
        assert_eq!(2, pipeline.intervals[1].blocks.len());

        // try to get blocks for download - max 1 interval
        let mut result = Vec::new();
        pipeline.collect_next_block_operations_to_download(1, &HashSet::default(), &mut result);
        assert_eq!(1, result.len());
        assert_eq!(result[0].as_ref(), block(2).as_ref());

        // try to get blocks for download - max 4
        let mut result = Vec::new();
        pipeline.collect_next_block_operations_to_download(4, &HashSet::default(), &mut result);
        assert_eq!(4, result.len());
        assert_eq!(result[0].as_ref(), block(2).as_ref());
        assert_eq!(result[1].as_ref(), block(5).as_ref());
        assert_eq!(result[2].as_ref(), block(8).as_ref());
        assert_eq!(result[3].as_ref(), block(10).as_ref());

        // try to get blocks for download - max 4 with ignored
        let mut result = Vec::new();
        pipeline.collect_next_block_operations_to_download(
            4,
            &hash_set![block(5), block(10)],
            &mut result,
        );
        assert_eq!(4, result.len());
        assert_eq!(result[0].as_ref(), block(2).as_ref());
        assert_eq!(result[1].as_ref(), block(8).as_ref());
        assert_eq!(result[2].as_ref(), block(13).as_ref());
        assert_eq!(result[3].as_ref(), block(15).as_ref());
    }

    #[test]
    fn test_download_all_blocks_and_operations() {
        // genesis
        let last_applied = block(0);
        // history blocks
        let history: Vec<Arc<BlockHash>> = vec![
            block(2),
            block(5),
            block(8),
            block(10),
            block(13),
            block(15),
            block(20),
        ];

        // create
        let mut pipeline = BranchState::new(last_applied, history, Arc::new(20));
        assert_eq!(pipeline.intervals.len(), 7);
        assert_interval(&pipeline.intervals[0], (block(0), block(2)));
        assert_interval(&pipeline.intervals[1], (block(2), block(5)));
        assert_interval(&pipeline.intervals[2], (block(5), block(8)));
        assert_interval(&pipeline.intervals[3], (block(8), block(10)));
        assert_interval(&pipeline.intervals[4], (block(10), block(13)));
        assert_interval(&pipeline.intervals[5], (block(13), block(15)));
        assert_interval(&pipeline.intervals[6], (block(15), block(20)));

        // download blocks and operations from 0 to 8
        pipeline.block_downloaded(
            &block(8),
            &InnerBlockState {
                block_downloaded: true,
                applied: false,
                operations_downloaded: true,
            },
            block(7),
        );
        pipeline.block_downloaded(
            &block(7),
            &InnerBlockState {
                block_downloaded: true,
                applied: false,
                operations_downloaded: true,
            },
            block(6),
        );
        pipeline.block_downloaded(
            &block(6),
            &InnerBlockState {
                block_downloaded: true,
                applied: false,
                operations_downloaded: true,
            },
            block(5),
        );
        pipeline.block_downloaded(
            &block(5),
            &InnerBlockState {
                block_downloaded: true,
                applied: false,
                operations_downloaded: true,
            },
            block(4),
        );
        pipeline.block_downloaded(
            &block(4),
            &InnerBlockState {
                block_downloaded: true,
                applied: false,
                operations_downloaded: true,
            },
            block(3),
        );
        pipeline.block_downloaded(
            &block(3),
            &InnerBlockState {
                block_downloaded: true,
                applied: false,
                operations_downloaded: true,
            },
            block(2),
        );
        pipeline.block_downloaded(
            &block(2),
            &InnerBlockState {
                block_downloaded: true,
                applied: false,
                operations_downloaded: true,
            },
            block(1),
        );
        pipeline.block_downloaded(
            &block(1),
            &InnerBlockState {
                block_downloaded: true,
                applied: false,
                operations_downloaded: true,
            },
            block(0),
        );

        // check all downloaded inervals
        assert!(pipeline.intervals[0].all_blocks_downloaded);
        assert!(pipeline.intervals[0].all_operations_downloaded);
        assert!(pipeline.intervals[1].all_blocks_downloaded);
        assert!(pipeline.intervals[1].all_operations_downloaded);
    }

    #[test]
    fn test_find_next_block_to_apply_batch() {
        // genesis
        let last_applied = block(0);
        // history blocks
        let history: Vec<Arc<BlockHash>> = vec![
            block(2),
            block(5),
            block(8),
            block(10),
            block(13),
            block(15),
            block(20),
        ];

        // create
        let mut pipeline = BranchState::new(last_applied, history, Arc::new(20));
        assert_eq!(pipeline.intervals.len(), 7);
        assert_interval(&pipeline.intervals[0], (block(0), block(2)));
        assert_interval(&pipeline.intervals[1], (block(2), block(5)));
        assert_interval(&pipeline.intervals[2], (block(5), block(8)));
        assert_interval(&pipeline.intervals[3], (block(8), block(10)));
        assert_interval(&pipeline.intervals[4], (block(10), block(13)));
        assert_interval(&pipeline.intervals[5], (block(13), block(15)));
        assert_interval(&pipeline.intervals[6], (block(15), block(20)));

        // download blocks and operations from 0 to 8
        pipeline.block_downloaded(
            &block(8),
            &InnerBlockState {
                block_downloaded: true,
                applied: false,
                operations_downloaded: true,
            },
            block(7),
        );
        pipeline.block_downloaded(
            &block(7),
            &InnerBlockState {
                block_downloaded: true,
                applied: false,
                operations_downloaded: true,
            },
            block(6),
        );
        pipeline.block_downloaded(
            &block(6),
            &InnerBlockState {
                block_downloaded: true,
                applied: false,
                operations_downloaded: true,
            },
            block(5),
        );
        pipeline.block_downloaded(
            &block(5),
            &InnerBlockState {
                block_downloaded: true,
                applied: false,
                operations_downloaded: true,
            },
            block(4),
        );
        pipeline.block_downloaded(
            &block(4),
            &InnerBlockState {
                block_downloaded: true,
                applied: false,
                operations_downloaded: true,
            },
            block(3),
        );
        pipeline.block_downloaded(
            &block(3),
            &InnerBlockState {
                block_downloaded: true,
                applied: false,
                operations_downloaded: true,
            },
            block(2),
        );
        pipeline.block_downloaded(
            &block(2),
            &InnerBlockState {
                block_downloaded: true,
                applied: false,
                operations_downloaded: true,
            },
            block(1),
        );
        pipeline.block_downloaded(
            &block(1),
            &InnerBlockState {
                block_downloaded: true,
                applied: false,
                operations_downloaded: true,
            },
            block(0),
        );

        // check all downloaded inervals
        assert!(pipeline.intervals[0].all_blocks_downloaded);
        assert!(pipeline.intervals[0].all_operations_downloaded);
        assert!(pipeline.intervals[1].all_blocks_downloaded);
        assert!(pipeline.intervals[1].all_operations_downloaded);

        // next for apply with max batch 0
        let next_batch = pipeline.find_next_block_to_apply(0);
        assert!(next_batch.is_some());
        let next_batch = next_batch.unwrap();
        assert_eq!(next_batch.block_to_apply.as_ref(), block(1).as_ref());
        assert_eq!(0, next_batch.successors_size());

        // next for apply with max batch 1
        let next_batch = pipeline.find_next_block_to_apply(1);
        assert!(next_batch.is_some());
        let next_batch = next_batch.unwrap();
        assert_eq!(next_batch.block_to_apply.as_ref(), block(1).as_ref());
        assert_eq!(1, next_batch.successors_size());
        assert_eq!(next_batch.successors[0].as_ref(), block(2).as_ref());

        // next for apply with max batch 100
        let next_batch = pipeline.find_next_block_to_apply(100);
        assert!(next_batch.is_some());
        let next_batch = next_batch.unwrap();
        assert_eq!(next_batch.block_to_apply.as_ref(), block(1).as_ref());
        assert_eq!(7, next_batch.successors_size());
        assert_eq!(next_batch.successors[0].as_ref(), block(2).as_ref());
        assert_eq!(next_batch.successors[1].as_ref(), block(3).as_ref());
        assert_eq!(next_batch.successors[2].as_ref(), block(4).as_ref());
        assert_eq!(next_batch.successors[3].as_ref(), block(5).as_ref());
        assert_eq!(next_batch.successors[4].as_ref(), block(6).as_ref());
        assert_eq!(next_batch.successors[5].as_ref(), block(7).as_ref());
        assert_eq!(next_batch.successors[6].as_ref(), block(8).as_ref());
    }

    fn assert_interval(
        tested: &BranchInterval,
        (expected_left, expected_right): (Arc<BlockHash>, Arc<BlockHash>),
    ) {
        assert_eq!(tested.blocks.len(), 2);
        assert_eq!(tested.blocks[0].block_hash.as_ref(), expected_left.as_ref());
        assert_eq!(
            tested.blocks[1].block_hash.as_ref(),
            expected_right.as_ref()
        );
    }
}
