mod cli;
mod common;

use std::path::PathBuf;

use directories::ProjectDirs;

use crate::common::log_ops;

// DHAT (profiling)
// #[cfg(feature = "dhat-heap")]
// #[global_allocator]
// static ALOC: dhat::Alloc = dhat::Alloc;

#[tokio::main]
async fn main() -> core::result::Result<(), Box<dyn std::error::Error>> {
    // DHAT (profiling)
    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();

    // XDG Base Directory Specification
    let project_dirs = ProjectDirs::from(
        "local", /*qualifier*/
        "cscs",  /*organization*/
        "manta", /*application*/
    );

    let mut path_to_manta_configuration_file = PathBuf::from(project_dirs.unwrap().config_dir());

    path_to_manta_configuration_file.push("config.toml"); // ~/.config/manta/config is the file

    log::info!(
        "Reading manta configuration from {}",
        &path_to_manta_configuration_file.to_string_lossy()
    );

    let settings = common::config_ops::get_configuration();

    // println!("settings:\n{:#?}", settings);

    let site_name = settings.get_string("site").unwrap();
    let site_detail_hashmap = settings.get_table("sites").unwrap();
    let site_detail_value = site_detail_hashmap
        .get(&site_name)
        .unwrap()
        .clone()
        .into_table()
        .unwrap();

    let site_available_vec = site_detail_hashmap.keys().cloned().collect::<Vec<String>>();

    // println!("site_detail_value:\n{:#?}", site_detail_value);

    let shasta_base_url = site_detail_value
        .get("shasta_base_url")
        .unwrap()
        .to_string();

    let keycloak_base_url = site_detail_value
        .get("keycloak_base_url")
        .unwrap()
        .to_string();

    let log_level = settings.get_string("log").unwrap_or("error".to_string());

    // Init logger
    // env_logger::init();
    // log4rs::init_file("log4rs.yml", Default::default()).unwrap(); // log4rs file configuration
    log_ops::configure(log_level); // log4rs programatically configuration

    if let Ok(socks_proxy) = settings.get_string("socks5_proxy") {
        std::env::set_var("SOCKS5", socks_proxy);
    }

    let settings_hsm_group_opt = settings.get_string("hsm_group").ok();

    /* let settings_hsm_available_vec = settings
    .get_array("hsm_available")
    .unwrap_or(Vec::new())
    .into_iter()
    .map(|hsm_group| hsm_group.into_string().unwrap())
    .collect::<Vec<String>>(); */

    let shasta_root_cert = common::config_ops::get_csm_root_cert_content(&site_name);

    let shasta_token = mesa::common::authentication::get_api_token(
        &shasta_base_url,
        &shasta_root_cert,
        &keycloak_base_url,
    )
    .await?;

    // Process input params
    let matches = crate::cli::build::build_cli(settings_hsm_group_opt.as_ref()).get_matches();
    let cli_result = crate::cli::process::process_cli(
        matches,
        &shasta_token,
        &shasta_base_url,
        &shasta_root_cert,
        settings_hsm_group_opt.as_ref(),
    )
    .await;

    match cli_result {
        Ok(_) => Ok(()),
        Err(e) => panic!("{}", e),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    pub fn test_prime_factor() {
        // let my_number_vec = vec![4, 6, 8, 12, 16, 20, 24];
        primefactor::PrimeFactors::from(4).to_vec();
    }
}
