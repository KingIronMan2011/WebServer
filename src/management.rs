//! Shared management operations used by both the CLI and the admin API.
//!
//! Keeping the mutations here prevents the two administration surfaces from
//! drifting in validation and persistence behaviour.

use crate::{
    config::{Config, RouteConfig},
    error::Result,
};

pub fn add_site(config: &mut Config, host: String) -> Result<()> {
    config.add_site(host)?;
    config.save()
}

pub fn remove_site(config: &mut Config, host: &str) -> Result<()> {
    config.remove_site(host)?;
    config.save()
}

pub fn create_site(config: &mut Config, host: String, route: RouteConfig) -> Result<()> {
    config.add_site(host)?;
    let host = config.sites.last().expect("site was added").host.clone();
    config.add_route(&host, route)?;
    config.validate()?;
    config.save()
}

pub fn add_route(config: &mut Config, host: &str, route: RouteConfig) -> Result<()> {
    config.add_route(host, route)?;
    config.validate()?;
    config.save()
}

pub fn remove_route(config: &mut Config, host: &str, path_prefix: &str) -> Result<()> {
    config.remove_route(host, path_prefix)?;
    config.validate()?;
    config.save()
}
