// Copyright (c) SimpleStaking, Viable Systems and Tezedge Contributors
// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

use crypto::hash::ProtocolHash;
use tezos_encoding::encoding::HasEncoding;
use tezos_encoding::nom::NomReader;

use super::limits::{GET_PROTOCOLS_MAX_LENGTH, PROTOCOL_COMPONENT_MAX_SIZE};

#[derive(Serialize, Deserialize, Debug, Clone, HasEncoding, NomReader)]
pub struct ProtocolMessage {
    protocol: Protocol,
}

impl ProtocolMessage {
    pub fn new(protocol: Protocol) -> Self {
        Self { protocol }
    }

    pub fn protocol(&self) -> &Protocol {
        &self.protocol
    }

}
// -----------------------------------------------------------------------------------------------
#[derive(Serialize, Deserialize, Debug, Clone, HasEncoding, NomReader)]
pub struct Component {
    name: String,
    interface: Option<String>,
    implementation: String,
}

impl Component {
    pub fn new(
        name: String,
        interface: Option<String>, 
        implementation: String
    ) -> Self {
        Self { name, interface, implementation }
    }

    pub fn name(&self) -> &String {
        &self.name
    }

    pub fn interface(&self) -> &Option<String> {
        &self.interface
    }

    pub fn implementation(&self) -> &String {
        &self.implementation
    }

}
// -----------------------------------------------------------------------------------------------
#[derive(Serialize, Deserialize, Debug, Clone, HasEncoding, NomReader)]
pub struct Protocol {
    expected_env_version: i16,
    #[encoding(dynamic = "PROTOCOL_COMPONENT_MAX_SIZE", list)]
    components: Vec<Component>,
}

impl Protocol {
    pub fn expected_env_version(&self) -> i16 {
        self.expected_env_version
    }

    pub fn components(&self) -> &Vec<Component> {
        &self.components
    }

    pub fn new(expected_env_version: i16, components: Vec<Component>) -> Self {
        Self { expected_env_version, components }
    }
}

// -----------------------------------------------------------------------------------------------
#[derive(Serialize, Deserialize, Debug, Clone, HasEncoding, NomReader)]
pub struct GetProtocolsMessage {
    #[encoding(dynamic, list = "GET_PROTOCOLS_MAX_LENGTH")]
    get_protocols: Vec<ProtocolHash>,
}

impl GetProtocolsMessage {
    pub fn new(get_protocols: Vec<ProtocolHash>) -> Self {
        Self { get_protocols }
    }
}
