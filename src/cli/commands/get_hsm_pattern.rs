use std::{collections::HashMap, sync::Arc, time::Instant};

use tokio::sync::Semaphore;

use crate::cli::commands::get_nodes_artifacts::NodeSummary;

pub async fn exec(
    shasta_token: &str,
    shasta_base_url: &str,
    shasta_root_cert: &[u8],
    hsm_group_name: &str,
) {
    // Target HSM group
    let hsm_group_value = mesa::hsm::group::shasta::http_client::get_hsm_group(
        shasta_token,
        shasta_base_url,
        shasta_root_cert,
        hsm_group_name,
    )
    .await
    .unwrap();

    log::info!(
        "Get HW artifacts for nodes in HSM group '{:?}' and members {:?}",
        hsm_group_value["label"],
        hsm_group_value["members"]
    );

    // Get target HSM group members
    let hsm_group_target_members =
        mesa::hsm::group::shasta::utils::get_member_vec_from_hsm_group_value(&hsm_group_value);

    let mut hsm_summary = Vec::new();

    let start_total = Instant::now();

    let mut tasks = tokio::task::JoinSet::new();

    let sem = Arc::new(Semaphore::new(5)); // CSM 1.3.1 higher number of concurrent tasks won't
                                           // make it faster

    // Get HW inventory details for target HSM group
    for hsm_member in hsm_group_target_members.clone() {
        let shasta_token_string = shasta_token.to_string(); // TODO: make it static
        let shasta_base_url_string = shasta_base_url.to_string(); // TODO: make it static
        let shasta_root_cert_vec = shasta_root_cert.to_vec();
        let hsm_member_string = hsm_member.to_string(); // TODO: make it static
                                                        //
        let permit = Arc::clone(&sem).acquire_owned().await;

        log::info!("Getting HW inventory details for node '{}'", hsm_member);
        tasks.spawn(async move {
            let _permit = permit; // Wait semaphore to allow new tasks https://github.com/tokio-rs/tokio/discussions/2648#discussioncomment-34885
            mesa::hsm::hw_inventory::shasta::http_client::get_hw_inventory(
                &shasta_token_string,
                &shasta_base_url_string,
                &shasta_root_cert_vec,
                &hsm_member_string,
            )
            .await
            .unwrap()
        });
    }

    while let Some(message) = tasks.join_next().await {
        if let Ok(mut node_hw_inventory) = message {
            node_hw_inventory = node_hw_inventory.pointer("/Nodes/0").unwrap().clone();
            let node_summary = NodeSummary::from_csm_value(node_hw_inventory.clone());
            hsm_summary.push(node_summary);
        } else {
            log::error!("Failed procesing/fetching node hw information");
        }
    }

    let duration = start_total.elapsed();

    log::info!(
        "Time elapsed in http calls to get hw inventory for HSM '{}' is: {:?}",
        hsm_group_name,
        duration
    );

    // println!("DEBUG - hsm_summary: {:#?}", hsm_summary);

    let mut hsm_node_hw_component_count_hashmap: HashMap<String, usize> = HashMap::new();

    for node_summary in hsm_summary {
        for processor in node_summary.processors {
            hsm_node_hw_component_count_hashmap
                .entry(
                    processor
                        .info
                        .unwrap()
                        .chars()
                        .filter(|c| !c.is_whitespace())
                        .collect(),
                )
                .and_modify(|qty| *qty += 1)
                .or_insert(1);
        }

        for node_accel in node_summary.node_accels {
            hsm_node_hw_component_count_hashmap
                .entry(
                    node_accel
                        .info
                        .unwrap()
                        .chars()
                        .filter(|c| !c.is_whitespace())
                        .collect(),
                )
                .and_modify(|qty| *qty += 1)
                .or_insert(1);
        }

        for memory_dimm in node_summary.memory {
            let memory_capacity = memory_dimm
                .clone()
                .info
                .unwrap_or("0".to_string())
                .split(" ")
                .collect::<Vec<_>>()
                .first()
                .unwrap()
                .to_string()
                .parse::<usize>()
                .unwrap();

            hsm_node_hw_component_count_hashmap
                .entry("memory".to_string())
                .and_modify(|memory_capacity_aux| *memory_capacity_aux += memory_capacity)
                .or_insert(memory_capacity);
        }
    }

    println!(
        "{}:{}",
        hsm_group_name,
        hsm_node_hw_component_count_hashmap
            .iter()
            .map(|(hw_component, qty)| format!("{}:{}", hw_component, qty))
            .collect::<Vec<String>>()
            .join(":")
    );
}
