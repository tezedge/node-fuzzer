// Copyright (c) SimpleStaking and Tezedge Contributors
// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

use crypto::hash::{HashType, ProtocolHash};
use tezos_encoding::encoding::{Encoding, Field, HasEncoding};
use tezos_encoding::has_encoding;

use crate::p2p::binary_message::cache::{BinaryDataCache, CachedData, CacheReader, CacheWriter};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProtocolMessage {
    protocol: Protocol,

    #[serde(skip_serializing)]
    body: BinaryDataCache,
}

has_encoding!(ProtocolMessage, PROTOCOL_MESSAGE_ENCODING, {
        Encoding::Obj(vec![
            Field::new("protocol", Protocol::encoding().clone())
        ])
});

impl CachedData for ProtocolMessage {
    #[inline]
    fn cache_reader(&self) -> & dyn CacheReader {
        &self.body
    }

    #[inline]
    fn cache_writer(&mut self) -> Option<&mut dyn CacheWriter> {
        Some(&mut self.body)
    }
}

// -----------------------------------------------------------------------------------------------
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Component {
    name: String,
    interface: Option<String>,
    implementation: String,

    #[serde(skip_serializing)]
    body: BinaryDataCache,
}

has_encoding!(Component, COMPONENT_ENCODING, {
        Encoding::Obj(vec![
            Field::new("name", Encoding::String),
            Field::new("interface", Encoding::option_field(Encoding::String)),
            Field::new("implementation", Encoding::String),
        ])
});

impl CachedData for Component {
    #[inline]
    fn cache_reader(&self) -> & dyn CacheReader {
        &self.body
    }

    #[inline]
    fn cache_writer(&mut self) -> Option<&mut dyn CacheWriter> {
        Some(&mut self.body)
    }
}

// -----------------------------------------------------------------------------------------------
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Protocol {
    expected_env_version: i16,
    components: Vec<Component>,

    #[serde(skip_serializing)]
    body: BinaryDataCache,
}

impl Protocol {
    pub fn expected_env_version(&self) -> i16 {
        self.expected_env_version
    }

    pub fn components(&self) -> &Vec<Component> {
        &self.components
    }
}

has_encoding!(Protocol, PROTOCOL_ENCODING, {
        Encoding::Obj(vec![
            Field::new("expected_env_version", Encoding::Int16),
            Field::new("components", Encoding::dynamic(Encoding::list(Component::encoding().clone())))
        ])
});

impl CachedData for Protocol {
    #[inline]
    fn cache_reader(&self) -> & dyn CacheReader {
        &self.body
    }

    #[inline]
    fn cache_writer(&mut self) -> Option<&mut dyn CacheWriter> {
        Some(&mut self.body)
    }
}

// -----------------------------------------------------------------------------------------------
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GetProtocolsMessage {
    get_protocols: Vec<ProtocolHash>,

    #[serde(skip_serializing)]
    body: BinaryDataCache,
}

has_encoding!(GetProtocolsMessage, GET_PROTOCOLS_MESSAGE_ENCODING, {
        Encoding::Obj(vec![
            Field::new("get_protocols", Encoding::dynamic(Encoding::list(Encoding::Hash(HashType::ProtocolHash)))),
        ])
});

impl CachedData for GetProtocolsMessage {
    #[inline]
    fn cache_reader(&self) -> & dyn CacheReader {
        &self.body
    }

    #[inline]
    fn cache_writer(&mut self) -> Option<&mut dyn CacheWriter> {
        Some(&mut self.body)
    }
}