// Copyright (c) SimpleStaking and Tezedge Contributors
// SPDX-License-Identifier: MIT

//! PeerBranchBootstrapper is actor, which is responsible to download branches from peers
//! and schedule downloaded blocks for block application.
//! PeerBranchBootstrapper operates just for one chain_id.

use std::sync::Arc;
use std::time::Duration;

use riker::actors::*;
use slog::{info, warn, Logger};

use crypto::hash::{BlockHash, ChainId};
use networking::PeerId;
use tezos_messages::p2p::encoding::block_header::Level;

use crate::shell_channel::ShellChannelRef;
use crate::state::bootstrap_state::{BootstrapState, InnerBlockState};
use crate::state::data_requester::DataRequesterRef;
use crate::state::peer_state::DataQueues;
use crate::subscription::subscribe_to_actor_terminated;

/// After this timeout peer will be disconnected if no activity is done on any pipeline
/// So if peer does not change any branch bootstrap, we will disconnect it
const STALE_BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(60 * 3);

/// If we have empty bootstrap pipelines for along time, we disconnect peer, means, peer is not provoding us a new current heads/branches
const MISSING_NEW_BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(60 * 2);

/// Constatnt for rescheduling of processing bootstrap pipelines
const SCHEDULE_ONE_TIMER_DELAY: Duration = Duration::from_secs(1);

/// How often to print stats in logs
const LOG_INTERVAL: Duration = Duration::from_secs(60);

/// Message commands [`PeerBranchBootstrapper`] to disconnect peer if any of bootstraping pipelines are stalled
#[derive(Clone, Debug)]
pub struct DisconnectStalledBootstraps {
    timeout: Duration,
}

#[derive(Clone, Debug)]
pub struct CleanPeerData(pub Arc<ActorUri>);

#[derive(Clone, Debug)]
pub struct LogStats;

#[derive(Clone, Debug)]
pub struct StartBranchBootstraping {
    peer_id: Arc<PeerId>,
    peer_queues: Arc<DataQueues>,
    chain_id: Arc<ChainId>,
    last_applied_block: Arc<BlockHash>,
    missing_history: Vec<Arc<BlockHash>>,
    to_level: Arc<Level>,
}

impl StartBranchBootstraping {
    pub fn new(
        peer_id: Arc<PeerId>,
        peer_queues: Arc<DataQueues>,
        chain_id: Arc<ChainId>,
        last_applied_block: Arc<BlockHash>,
        missing_history: Vec<Arc<BlockHash>>,
        to_level: Arc<Level>,
    ) -> Self {
        Self {
            peer_id,
            peer_queues,
            chain_id,
            last_applied_block,
            missing_history,
            to_level,
        }
    }
}

/// This message should be trriggered, when all operations for the block are downloaded
#[derive(Clone, Debug)]
pub struct UpdateOperationsState {
    block_hash: BlockHash,
}

impl UpdateOperationsState {
    pub fn new(block_hash: BlockHash) -> Self {
        Self { block_hash }
    }
}

#[derive(Clone, Debug)]
pub struct UpdateBlockState {
    block_hash: BlockHash,
    predecessor_block_hash: BlockHash,
    new_state: InnerBlockState,
}

impl UpdateBlockState {
    pub fn new(
        block_hash: BlockHash,
        predecessor_block_hash: BlockHash,
        new_state: InnerBlockState,
    ) -> Self {
        Self {
            block_hash,
            predecessor_block_hash,
            new_state,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PingBootstrapPipelinesProcessing;

/// Event is fired, when some batch was finished, so next can go
#[derive(Clone, Debug)]
pub struct ApplyBlockBatchDone {
    pub last_applied: Arc<BlockHash>,
}

/// Event is fired, when some batch was not applied and error occured
#[derive(Clone, Debug)]
pub struct ApplyBlockBatchFailed {
    pub failed_block: Arc<BlockHash>,
}

#[actor(
    StartBranchBootstraping,
    PingBootstrapPipelinesProcessing,
    UpdateBlockState,
    UpdateOperationsState,
    ApplyBlockBatchDone,
    ApplyBlockBatchFailed,
    DisconnectStalledBootstraps,
    CleanPeerData,
    LogStats,
    SystemEvent
)]
pub struct PeerBranchBootstrapper {
    chain_id: Arc<ChainId>,

    shell_channel: ShellChannelRef,
    requester: DataRequesterRef,

    bootstrap_state: BootstrapState,

    cfg: PeerBranchBootstrapperConfiguration,
}

#[derive(Clone)]
pub struct PeerBranchBootstrapperConfiguration {
    max_bootstrap_branches_per_peer: usize,
    max_block_apply_batch: usize,
}

impl PeerBranchBootstrapperConfiguration {
    pub fn new(max_bootstrap_branches_per_peer: usize, max_block_apply_batch: usize) -> Self {
        Self {
            max_bootstrap_branches_per_peer,
            max_block_apply_batch,
        }
    }
}

pub type PeerBranchBootstrapperRef = ActorRef<PeerBranchBootstrapperMsg>;

impl PeerBranchBootstrapper {
    /// Create new actor instance.
    pub fn actor(
        sys: &ActorSystem,
        chain_id: Arc<ChainId>,
        requester: DataRequesterRef,
        shell_channel: ShellChannelRef,
        cfg: PeerBranchBootstrapperConfiguration,
    ) -> Result<PeerBranchBootstrapperRef, CreateError> {
        sys.actor_of_props::<PeerBranchBootstrapper>(
            &format!("peer-branch-bootstrapper-{}", &chain_id.to_base58_check()),
            Props::new_args((chain_id, requester, shell_channel, cfg)),
        )
    }

    /// We can receive parallely diferent event, and after each event we dont need to schedule this one, becuase it is unnecesseray:
    ///
    /// e1 (schedule pbpb1), e2 (schedule pbpb2), e3 (schedule pbpb3), pbpb1, pbpb2, pbpb3, e4 (schedule pbpb4), pbpb4
    ///
    /// we want it like this:
    ///
    /// e1 (schedule pbpb1), e2 (schedule pbpb2 - not needed), e3 (schedule pbpb3 - not needed), pbpb1, e4 (schedule pbpb4), pbpb4
    ///
    fn schedule_process_bootstrap_pipelines(
        &mut self,
        ctx: &Context<PeerBranchBootstrapperMsg>,
        schedule_at_delay: Duration,
    ) {
        ctx.schedule_once(
            schedule_at_delay,
            ctx.myself(),
            None,
            PingBootstrapPipelinesProcessing,
        );
    }

    fn clean_peer_data(&mut self, actor: &ActorUri) {
        self.bootstrap_state.clean_peer_data(actor);
    }

    fn process_bootstrap_pipelines(
        &mut self,
        ctx: &Context<PeerBranchBootstrapperMsg>,
        log: &Logger,
    ) {
        let PeerBranchBootstrapper {
            bootstrap_state,
            requester,
            cfg,
            chain_id,
            shell_channel,
            ..
        } = self;

        // schedule missing blocks for download
        bootstrap_state.schedule_blocks_to_download(requester, log);

        // schedule missing operations for download
        bootstrap_state.schedule_operations_to_download(requester, log);

        // schedule missing operations for download
        bootstrap_state.schedule_blocks_for_apply(
            requester,
            cfg.max_block_apply_batch,
            chain_id,
            &ctx.myself,
        );

        // check, if we completed any pipeline
        bootstrap_state.check_bootstrapped_branches(shell_channel, log);
    }
}

impl
    ActorFactoryArgs<(
        Arc<ChainId>,
        DataRequesterRef,
        ShellChannelRef,
        PeerBranchBootstrapperConfiguration,
    )> for PeerBranchBootstrapper
{
    fn create_args(
        (chain_id, requester, shell_channel, cfg): (
            Arc<ChainId>,
            DataRequesterRef,
            ShellChannelRef,
            PeerBranchBootstrapperConfiguration,
        ),
    ) -> Self {
        PeerBranchBootstrapper {
            chain_id,
            requester,
            shell_channel,
            bootstrap_state: Default::default(),
            cfg,
        }
    }
}

impl Actor for PeerBranchBootstrapper {
    type Msg = PeerBranchBootstrapperMsg;

    fn pre_start(&mut self, ctx: &Context<Self::Msg>) {
        subscribe_to_actor_terminated(ctx.system.sys_events(), ctx.myself());

        ctx.schedule::<Self::Msg, _>(
            STALE_BOOTSTRAP_TIMEOUT,
            STALE_BOOTSTRAP_TIMEOUT,
            ctx.myself(),
            None,
            DisconnectStalledBootstraps {
                timeout: STALE_BOOTSTRAP_TIMEOUT,
            }
            .into(),
        );

        ctx.schedule::<Self::Msg, _>(
            LOG_INTERVAL / 2,
            LOG_INTERVAL,
            ctx.myself(),
            None,
            LogStats.into(),
        );
    }

    fn post_start(&mut self, ctx: &Context<Self::Msg>) {
        info!(ctx.system.log(), "Peer branch bootstrapped started";
                                "chain_id" => self.chain_id.to_base58_check());
    }

    fn recv(&mut self, ctx: &Context<Self::Msg>, msg: Self::Msg, sender: Sender) {
        self.receive(ctx, msg, sender);
    }

    fn sys_recv(
        &mut self,
        ctx: &Context<Self::Msg>,
        msg: SystemMsg,
        sender: Option<BasicActorRef>,
    ) {
        if let SystemMsg::Event(evt) = msg {
            self.receive(ctx, evt, sender);
        }
    }
}

impl Receive<SystemEvent> for PeerBranchBootstrapper {
    type Msg = PeerBranchBootstrapperMsg;

    fn receive(&mut self, _: &Context<Self::Msg>, msg: SystemEvent, _: Option<BasicActorRef>) {
        if let SystemEvent::ActorTerminated(evt) = msg {
            self.clean_peer_data(evt.actor.uri());
        }
    }
}

impl Receive<StartBranchBootstraping> for PeerBranchBootstrapper {
    type Msg = PeerBranchBootstrapperMsg;

    fn receive(
        &mut self,
        ctx: &Context<Self::Msg>,
        msg: StartBranchBootstraping,
        _: Option<BasicActorRef>,
    ) {
        let log = ctx.system.log();
        info!(log, "Start branch bootstrapping process";
            "last_applied_block" => msg.last_applied_block.to_base58_check(),
            "missing_history" => msg.missing_history
                .iter()
                .map(|b| b.to_base58_check())
                .collect::<Vec<String>>()
                .join(", "),
            "to_level" => &msg.to_level,
            "peer_id" => msg.peer_id.peer_id_marker.clone(), "peer_ip" => msg.peer_id.peer_address.to_string(), "peer" => msg.peer_id.peer_ref.name(), "peer_uri" => msg.peer_id.peer_ref.uri().to_string(),
        );

        // bootstrapper supports just one chain, if this will be issue, we need to create a new bootstrapper per chain_id
        if !self.chain_id.eq(&msg.chain_id) {
            warn!(log, "Branch is rejected, because of different chain_id";
                "peer_branch_bootstrapper_chain_id" => self.chain_id.to_base58_check(),
                "requested_branch_chain_id" => msg.chain_id.to_base58_check(),
                "last_applied_block" => msg.last_applied_block.to_base58_check(),
                "peer_id" => msg.peer_id.peer_id_marker.clone(), "peer_ip" => msg.peer_id.peer_address.to_string(), "peer" => msg.peer_id.peer_ref.name(), "peer_uri" => msg.peer_id.peer_ref.uri().to_string(),
            );
            return;
        }

        // add new branch (if possible)
        self.bootstrap_state.add_new_branch(
            msg.peer_id,
            msg.peer_queues,
            msg.last_applied_block,
            msg.missing_history,
            msg.to_level,
            self.cfg.max_bootstrap_branches_per_peer,
            &log,
        );

        // process
        self.process_bootstrap_pipelines(ctx, &log)
    }
}

impl Receive<PingBootstrapPipelinesProcessing> for PeerBranchBootstrapper {
    type Msg = PeerBranchBootstrapperMsg;

    fn receive(
        &mut self,
        ctx: &Context<Self::Msg>,
        _: PingBootstrapPipelinesProcessing,
        _: Option<BasicActorRef>,
    ) {
        self.process_bootstrap_pipelines(ctx, &ctx.system.log());
    }
}

impl Receive<CleanPeerData> for PeerBranchBootstrapper {
    type Msg = PeerBranchBootstrapperMsg;

    fn receive(&mut self, _: &Context<Self::Msg>, msg: CleanPeerData, _: Option<BasicActorRef>) {
        self.clean_peer_data(&msg.0);
    }
}

impl Receive<LogStats> for PeerBranchBootstrapper {
    type Msg = PeerBranchBootstrapperMsg;

    fn receive(&mut self, ctx: &Context<Self::Msg>, _: LogStats, _: Sender) {
        let (
            processing_blocks,
            (
                processing_blocks_downloaded,
                processing_blocks_operations_downloaded,
                processing_blocks_applied,
                processing_blocks_scheduled_for_apply,
            ),
        ) = self.bootstrap_state.blocks_stats();
        let (
            processing_block_intervals,
            (
                processing_block_intervals_downloaded,
                processing_block_intervals_operations_downloaded,
            ),
        ) = self.bootstrap_state.block_intervals_stats();

        info!(ctx.system.log(), "Peer branch bootstrapper processing info";
                   "peers_count" => self.bootstrap_state.peers_count(),
                   "processing_block_intervals" => processing_block_intervals,
                   "processing_block_intervals_downloaded" => processing_block_intervals_downloaded,
                   "processing_block_intervals_operations_downloaded" => processing_block_intervals_operations_downloaded,
                   "processing_blocks" => processing_blocks,
                   "processing_blocks_downloaded" => processing_blocks_downloaded,
                   "processing_blocks_operations_downloaded" => processing_blocks_operations_downloaded,
                   "processing_blocks_applied" => processing_blocks_applied,
                   "processing_blocks_scheduled_for_apply" => processing_blocks_scheduled_for_apply,
        );
    }
}

impl Receive<UpdateBlockState> for PeerBranchBootstrapper {
    type Msg = PeerBranchBootstrapperMsg;

    fn receive(
        &mut self,
        ctx: &Context<Self::Msg>,
        msg: UpdateBlockState,
        _: Option<BasicActorRef>,
    ) {
        // process message
        let UpdateBlockState {
            block_hash,
            predecessor_block_hash,
            new_state,
        } = msg;
        self.bootstrap_state
            .block_downloaded(block_hash, predecessor_block_hash, new_state);

        // process bootstrap
        self.process_bootstrap_pipelines(ctx, &ctx.system.log())
    }
}

impl Receive<UpdateOperationsState> for PeerBranchBootstrapper {
    type Msg = PeerBranchBootstrapperMsg;

    fn receive(
        &mut self,
        ctx: &Context<Self::Msg>,
        msg: UpdateOperationsState,
        _: Option<BasicActorRef>,
    ) {
        // process message
        self.bootstrap_state
            .block_operations_downloaded(msg.block_hash);

        // process
        self.process_bootstrap_pipelines(ctx, &ctx.system.log())
    }
}

impl Receive<ApplyBlockBatchDone> for PeerBranchBootstrapper {
    type Msg = PeerBranchBootstrapperMsg;

    fn receive(
        &mut self,
        ctx: &Context<Self::Msg>,
        msg: ApplyBlockBatchDone,
        _: Option<BasicActorRef>,
    ) {
        // process message
        self.bootstrap_state.block_applied(&msg.last_applied);

        // process
        self.process_bootstrap_pipelines(ctx, &ctx.system.log())
    }
}

impl Receive<ApplyBlockBatchFailed> for PeerBranchBootstrapper {
    type Msg = PeerBranchBootstrapperMsg;

    fn receive(
        &mut self,
        ctx: &Context<Self::Msg>,
        msg: ApplyBlockBatchFailed,
        _: Option<BasicActorRef>,
    ) {
        self.bootstrap_state
            .block_apply_failed(&msg.failed_block, &ctx.system.log());
    }
}

impl Receive<DisconnectStalledBootstraps> for PeerBranchBootstrapper {
    type Msg = PeerBranchBootstrapperMsg;

    fn receive(
        &mut self,
        ctx: &Context<Self::Msg>,
        msg: DisconnectStalledBootstraps,
        _: Option<BasicActorRef>,
    ) {
        let log = ctx.system.log();

        let PeerBranchBootstrapper {
            bootstrap_state,
            shell_channel,
            ..
        } = self;

        bootstrap_state.check_bootstrapped_branches(shell_channel, &log);
        bootstrap_state.check_stalled_peers(
            msg.timeout,
            &MISSING_NEW_BOOTSTRAP_TIMEOUT,
            &log,
            |peer| {
                ctx.system.stop(peer.peer_ref.clone());
            },
        );

        self.schedule_process_bootstrap_pipelines(ctx, SCHEDULE_ONE_TIMER_DELAY);
    }
}
