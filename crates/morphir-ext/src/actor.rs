//! Actor wrapper for Morphir extensions using Kameo.

use kameo::Actor;
use kameo::message::{Context, Message};
use anyhow::Result;

use crate::{
    ExtensionInstance, ExtensionRuntime, 
    WitEnvelope, EnvValue,
};

/// Message to initialize the extension.
#[derive(Debug)]
pub struct InitMsg {
    pub init_data: WitEnvelope,
}

/// Message to update the extension state.
#[derive(Debug)]
pub struct UpdateMsg {
    pub msg: WitEnvelope,
    pub model: WitEnvelope,
}

/// Message to get subscriptions.
#[derive(Debug)]
pub struct SubscriptionsMsg {
    pub model: WitEnvelope,
}

/// Message to get extension info.
#[derive(Debug)]
pub struct InfoMsg;

/// Message to set an environment variable.
#[derive(Debug)]
pub struct SetEnvVarMsg {
    pub name: String,
    pub value: EnvValue,
}

/// Message to get an environment variable.
#[derive(Debug)]
pub struct GetEnvVarMsg {
    pub name: String,
}

/// Result of init or update: (model, commands)
#[derive(Debug)]
pub struct ModelCommands {
    pub model: WitEnvelope,
    pub commands: WitEnvelope,
}

/// Actor that wraps an extension instance.
#[derive(Actor)]
pub struct ExtensionActor {
    instance: ExtensionInstance,
}

impl ExtensionActor {
    /// Create a new ExtensionActor from an already instantiated extension.
    pub fn new(instance: ExtensionInstance) -> Self {
        Self { instance }
    }
    
    /// Create a new ExtensionActor by loading a component from a file.
    pub fn from_file(runtime: &ExtensionRuntime, path: impl AsRef<std::path::Path>) -> Result<Self> {
        let component = runtime.load_component(path)?;
        let instance = runtime.instantiate(&component)?;
        Ok(Self::new(instance))
    }
    
    /// Create a new ExtensionActor by loading a component from bytes.
    pub fn from_bytes(runtime: &ExtensionRuntime, bytes: &[u8]) -> Result<Self> {
        let component = runtime.load_component_from_bytes(bytes)?;
        let instance = runtime.instantiate(&component)?;
        Ok(Self::new(instance))
    }
}

// Message handler for Init
impl Message<InitMsg> for ExtensionActor {
    type Reply = Result<ModelCommands>;
    
    async fn handle(
        &mut self,
        msg: InitMsg,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let (model, commands) = self.instance.init(msg.init_data)?;
        Ok(ModelCommands { model, commands })
    }
}

// Message handler for Update
impl Message<UpdateMsg> for ExtensionActor {
    type Reply = Result<ModelCommands>;
    
    async fn handle(
        &mut self,
        msg: UpdateMsg,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let (model, commands) = self.instance.update(msg.msg, msg.model)?;
        Ok(ModelCommands { model, commands })
    }
}

// Message handler for Subscriptions
impl Message<SubscriptionsMsg> for ExtensionActor {
    type Reply = Result<WitEnvelope>;
    
    async fn handle(
        &mut self,
        msg: SubscriptionsMsg,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.instance.subscriptions(msg.model)
    }
}

// Message handler for Info
impl Message<InfoMsg> for ExtensionActor {
    type Reply = Result<WitEnvelope>;
    
    async fn handle(
        &mut self,
        _msg: InfoMsg,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.instance.info()
    }
}

// Message handler for SetEnvVar
impl Message<SetEnvVarMsg> for ExtensionActor {
    type Reply = ();
    
    async fn handle(
        &mut self,
        msg: SetEnvVarMsg,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.instance.set_env_var(msg.name, msg.value);
    }
}

// Message handler for GetEnvVar
impl Message<GetEnvVarMsg> for ExtensionActor {
    type Reply = Option<EnvValue>;
    
    async fn handle(
        &mut self,
        msg: GetEnvVarMsg,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.instance.get_env_var(&msg.name).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_init_msg_debug() {
        // Just test that types are debuggable
        let _ = format!("{:?}", InfoMsg);
    }
}
