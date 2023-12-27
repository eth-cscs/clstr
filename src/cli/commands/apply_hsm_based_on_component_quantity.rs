use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc, time::Instant};
use tokio::sync::Semaphore;

use crate::cli::commands::{
    apply_hsm_based_on_component_quantity::utils::{
        calculate_all_deltas, calculate_hsm_hw_component_count,
        calculate_hsm_hw_component_normalized_density_score_from_hsm_node_hw_component_count_vec,
        calculate_hsm_hw_component_normalized_node_density_score_downscale,
        calculate_hsm_total_number_hw_components, calculate_node_density_score,
        get_hsm_hw_component_count_filtered_by_user_request, get_node_hw_component_count,
        upscale_node_migration,
    },
    get_hsm_artifacts::print_table_f32_score,
};

// TEST --> cargo run -- a hsm -p zinal:a100:4:epyc:30:instinct:2
// TEST --> cargo run -- a hsm -p zinal:a100:3:epyc:3
//
/// Updates HSM groups following a string pattern like <hsm group name>:<node type>:<quantity>:<node type>:<quantity>:...,<hsm group name>:... or <hsm group>:<node type>*:<quantity> and <node type> being <prop>:<prop>:... or <prop>(<prop>)*
/// Example: zinal:a100:epyc:2:epyc:3:epyc:instinct:1 will provide a list of nodes for HSM group
/// zinal with x2 nodes which contains epic AND a100, x3 nodes with epyc, x1 node with epyc AND instinct.
///
/// Convention:
/// a100 - hw component
/// a100:epyc - hw profile
/// a100:epyc:5 - hw profile counter
///
/// NOTE: pattern > hw profile > hw property. pattern --> zinal:a100:epyc:2:epyc:instinct:8:epyc:25,
/// hw profile --> a100:epyc or epyc:instinct, hw property --> a100 or epyc or instinct
///
/// OPTION: nodes needs to be geographically nearby, we meassure this by calculating the "distance" between nodes.
/// The distance between 2 nodes is represented by a synbolic number which can be calculated by comparing the xnames of the nodes (which does not need increase/scale linearly, as shown in the examples below)
/// xXcCsSbB -- distance 0 (same blade)
/// xXcCsS ---- distance 1 (same slot)
/// xXcC ------ distance 2 (same chasis)
/// xX -------- distance 3 (same rack)
/// ----------- distance 4 (different rack)
///
/// OPTION 2: We need an option to say if we want to "keep" the nodes already in the HSM group which complains with the user/dessired pattern or if
///we want to "reallocate all nodes" because we want them to be geographically next to each other.
///
/// OPTION 3: We need an option to submit the changes to the CSM API to create the new HSM groups
///
/// EDGE CASES:
/// same pattern that identifies same hw type duplicated in same hsm group (eg
/// zinal:a100:epyc:2:epyc:instinct:1:epyc:a100:1) where node type a100:epyc is mentioned twice,
/// which one should we take? for now we will give an error
///
/// multiple HSM groups (eg zinal:a100:3 hohgant:epyc:4) and how should we express this? should we
/// separate each HSM group by ',' or ' ', something else?
///
/// duplicated HSM group in input (eg zinal:a100:epyc:4,zinal:a100:epyc1:instinct:epyc:1) what to
/// do here? we group both or we use the last one only or we use the first one only? for now we
/// will give an error
///
/// if HSM group looses all its members, then ask user if HSM should be deleted

// VARS

pub async fn exec(
    shasta_token: &str,
    shasta_base_url: &str,
    shasta_root_cert: &[u8],
    pattern: &str,
    parent_hsm_group_name: &str,
) {
    // lcm -> used to normalize and quantify memory capacity
    let mem_lcm = 16384; // 1024 * 16

    // Normalize text in lowercase and separate each HSM group hw inventory pattern
    let pattern_lowercase = pattern.to_lowercase();

    let mut pattern_element_vec: Vec<&str> = pattern_lowercase.split(':').collect();

    let target_hsm_group_name = pattern_element_vec.remove(0);

    let mut user_defined_hw_component_count_hashmap: HashMap<String, usize> = HashMap::new();

    // Check user input is correct
    for hw_component_counter in pattern_element_vec.chunks(2) {
        if hw_component_counter[0].parse::<String>().is_ok()
            && hw_component_counter[1].parse::<usize>().is_ok()
        {
            user_defined_hw_component_count_hashmap.insert(
                hw_component_counter[0].parse::<String>().unwrap(),
                hw_component_counter[1].parse::<usize>().unwrap(),
            );
        } else {
            log::error!("Error in pattern. Please make sure to follow <hsm name>:<hw component>:<counter>:... eg <tasna>:a100:4:epyc:10:instinct:8");
        }
    }

    println!(
        "User defined hw components with counters: {:?}",
        user_defined_hw_component_count_hashmap
    );

    let mut user_defined_hw_component_vec: Vec<String> = user_defined_hw_component_count_hashmap
        .keys()
        .cloned()
        .collect();

    user_defined_hw_component_vec.sort();

    // *********************************************************************************************************
    // PREPREQUISITES TARGET HSM GROUP

    // Get target HSM group details
    let hsm_group_target_value: Value = mesa::hsm::group::shasta::http_client::get(
        shasta_token,
        shasta_base_url,
        shasta_root_cert,
        Some(&target_hsm_group_name.to_string()),
    )
    .await
    .unwrap()
    .first()
    .unwrap_or(&json!({
        "label": target_hsm_group_name,
        "description": "",
        "members": {
            "ids": []
        }
    }))
    .clone();

    /* // If target HSM does not exists, then create a new one
    let hsm_group_target_value = match hsm_group_target_value_rslt {
        Err(_) => json!({
            "label": target_hsm_group_name,
            "description": "",
            "members": {
                "ids": []
            }
        }),
        Ok(hsm_group_target_value) => hsm_group_target_value,
    }; */

    // Get target HSM group members
    let hsm_group_target_members =
        mesa::hsm::group::shasta::utils::get_member_vec_from_hsm_group_value(
            &hsm_group_target_value,
        );

    // Get HSM group members hw configurfation based on user input
    let start = Instant::now();

    let mut tasks = tokio::task::JoinSet::new();

    let sem = Arc::new(Semaphore::new(5)); // CSM 1.3.1 higher number of concurrent tasks won't

    // List of node hw component counters belonging to target hsm group
    let mut target_hsm_node_hw_component_count_vec = Vec::new();

    // Get HW inventory details for target HSM group
    for hsm_member in hsm_group_target_members.clone() {
        let shasta_token_string = shasta_token.to_string(); // TODO: make it static
        let shasta_base_url_string = shasta_base_url.to_string(); // TODO: make it static
        let shasta_root_cert_vec = shasta_root_cert.to_vec();
        let user_defined_hw_component_vec = user_defined_hw_component_count_hashmap
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .clone();

        let permit = Arc::clone(&sem).acquire_owned().await;

        // println!("user_defined_hw_profile_vec_aux: {:?}", user_defined_hw_profile_vec_aux);
        tasks.spawn(async move {
            let _permit = permit; // Wait semaphore to allow new tasks https://github.com/tokio-rs/tokio/discussions/2648#discussioncomment-34885
            get_node_hw_component_count(
                shasta_token_string,
                shasta_base_url_string,
                shasta_root_cert_vec,
                &hsm_member,
                user_defined_hw_component_vec,
            )
            .await
        });
    }

    while let Some(message) = tasks.join_next().await {
        if let Ok(mut node_hw_component_vec_tuple) = message {
            node_hw_component_vec_tuple.1.sort();

            let mut node_hw_component_count_hashmap: HashMap<String, usize> = HashMap::new();

            for node_hw_property_vec in node_hw_component_vec_tuple.1 {
                let count = node_hw_component_count_hashmap
                    .entry(node_hw_property_vec)
                    .or_insert(0);
                *count += 1;
            }

            let node_memory_total_capacity: u64 = node_hw_component_vec_tuple.2.iter().sum();

            node_hw_component_count_hashmap.insert(
                "memory".to_string(),
                (node_memory_total_capacity / mem_lcm)
                    .try_into()
                    .unwrap_or(0),
            );

            target_hsm_node_hw_component_count_vec.push((
                node_hw_component_vec_tuple.0,
                node_hw_component_count_hashmap,
            ));
        } else {
            log::error!("Failed procesing/fetching node hw information");
        }
    }

    let duration = start.elapsed();
    log::info!(
        "Time elapsed to calculate actual_hsm_node_hw_profile_vec in '{}' is: {:?}",
        target_hsm_group_name,
        duration
    );

    // Sort nodes hw counters by node name
    target_hsm_node_hw_component_count_vec
        .sort_by_key(|target_hsm_group_hw_component| target_hsm_group_hw_component.0.clone());

    /* println!(
        "DEBUG - target_hsm_node_hw_component_count_vec: {:?}",
        target_hsm_node_hw_component_count_vec
    ); */

    // Calculate hw component counters in HSM
    // let target_hsm_hw_component_count_hashmap: HashMap<String, usize> =
    //     calculate_hsm_hw_component_count(&target_hsm_node_hw_component_count_vec);

    /* println!(
        "DEBUG - target_hsm_hw_component_count: {:?}",
        target_hsm_hw_component_count_hashmap
    ); */

    // Calculate hw component counters in HSM filtered by user request
    let target_hsm_hw_component_count_filtered_by_user_request_hashmap: HashMap<String, usize> =
        get_hsm_hw_component_count_filtered_by_user_request(
            &user_defined_hw_component_vec,
            &target_hsm_node_hw_component_count_vec,
        );

    println!(
        "HSM '{}' hw component counters filtered by user request: {:?}",
        target_hsm_group_name, target_hsm_hw_component_count_filtered_by_user_request_hashmap
    );

    /* // Calculate density scores for each node in HSM
    let target_hsm_density_score_hashmap: HashMap<String, usize> =
        calculate_node_density_score(&target_hsm_node_hw_component_count_vec); */

    /* println!(
        "DEBUG - HSM '{}' node's density score: {:?}",
        target_hsm_group_name, target_hsm_density_score_hashmap
    ); */

    // Calculate total number of hw components in HSM
    /* let target_hsm_total_number_hw_components: usize =
    calculate_hsm_total_number_hw_components(&mut target_hsm_node_hw_component_count_vec); */

    /* println!(
        "DEBUG - HSM '{}' --> total_number_hw_components: {}",
        target_hsm_group_name, target_hsm_total_number_hw_components
    ); */

    // Calculate nomarlized score for each hw component in HSM group
    /* let target_hsm_hw_component_normalized_scores =
    calculate_hsm_hw_component_normalized_density_score_from_hsm_node_hw_component_count_vec(
        &target_hsm_node_hw_component_count_vec,
        target_hsm_total_number_hw_components,
    ); */

    /* println!(
        "DEBUG - target_hsm_hw_component_normalized_scores: {:?}",
        target_hsm_hw_component_normalized_scores
    ); */

    // *********************************************************************************************************
    // PREREQUISITES PARENT HSM GROUP

    // Get parent HSM group details
    let hsm_group_parent_value = mesa::hsm::group::shasta::http_client::get(
        shasta_token,
        shasta_base_url,
        shasta_root_cert,
        Some(&parent_hsm_group_name.to_string()),
    )
    .await
    .unwrap()
    .first()
    .unwrap()
    .clone();

    // Get target HSM group members
    let hsm_group_parent_members =
        mesa::hsm::group::shasta::utils::get_member_vec_from_hsm_group_value(
            &hsm_group_parent_value,
        );

    // Get HSM group members hw configurfation based on user input
    let start = Instant::now();

    let mut tasks = tokio::task::JoinSet::new();

    let sem = Arc::new(Semaphore::new(5)); // CSM 1.3.1 higher number of concurrent tasks won't
                                           // make it faster

    // List of node hw component counters belonging to parent hsm group
    let mut parent_hsm_node_hw_component_count_vec = Vec::new();

    // Get HW inventory details for parent HSM group
    for hsm_member in hsm_group_parent_members.clone() {
        let shasta_token_string = shasta_token.to_string();
        let shasta_base_url_string = shasta_base_url.to_string();
        let shasta_root_cert_vec = shasta_root_cert.to_vec();
        let user_defined_hw_component_vec = user_defined_hw_component_count_hashmap
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .clone();

        let permit = Arc::clone(&sem).acquire_owned().await;

        // println!("user_defined_hw_profile_vec_aux: {:?}", user_defined_hw_profile_vec_aux);
        tasks.spawn(async move {
            let _permit = permit; // Wait semaphore to allow new tasks https://github.com/tokio-rs/tokio/discussions/2648#discussioncomment-34885
            get_node_hw_component_count(
                shasta_token_string,
                shasta_base_url_string,
                shasta_root_cert_vec,
                &hsm_member,
                user_defined_hw_component_vec,
            )
            .await
        });
    }

    while let Some(message) = tasks.join_next().await {
        if let Ok(mut node_hw_component_vec_tuple) = message {
            node_hw_component_vec_tuple.1.sort();

            let mut node_hw_component_count_hashmap: HashMap<String, usize> = HashMap::new();

            for node_hw_property_vec in node_hw_component_vec_tuple.1 {
                let count = node_hw_component_count_hashmap
                    .entry(node_hw_property_vec)
                    .or_insert(0);
                *count += 1;
            }

            let node_memory_total_capacity: u64 = node_hw_component_vec_tuple.2.iter().sum();

            node_hw_component_count_hashmap.insert(
                "memory".to_string(),
                (node_memory_total_capacity / mem_lcm)
                    .try_into()
                    .unwrap_or(0),
            );

            parent_hsm_node_hw_component_count_vec.push((
                node_hw_component_vec_tuple.0,
                node_hw_component_count_hashmap,
            ));
        } else {
            log::error!("Failed procesing/fetching node hw information");
        }
    }

    let duration = start.elapsed();
    log::info!(
        "Time elapsed to calculate actual_hsm_node_hw_profile_vec in '{}' is: {:?}",
        parent_hsm_group_name,
        duration
    );

    // Sort nodes hw counters by node name
    parent_hsm_node_hw_component_count_vec
        .sort_by_key(|parent_hsm_group_hw_component| parent_hsm_group_hw_component.0.clone());

    /* println!(
        "DEBUG - node_hw_component_count_hashmap_parent_hsm_vec: {:?}",
        parent_hsm_node_hw_component_count_vec
    ); */

    //CAN'T CALCULATE ANY SCORE, COUNT IN HSM GROUP BECUASE AT THIS POINT PARENT HSM HAS NOT
    //INCORPORATED THE NODES FROM PARENT HSM

    // *********************************************************************************************************
    // HSM UPDATES

    // Calculate deltas
    /* let (
        mut hw_components_to_migrate_from_target_hsm_to_parent_hsm,
        hw_components_to_migrate_from_parent_hsm_to_target_hsm,
    ) = calculate_all_deltas(
        &user_defined_hw_component_count_hashmap,
        &target_hsm_hw_component_count_filtered_by_user_request_hashmap,
    ); */

    // Add hw components in HSM target nodes not requested by user to downscaling deltas (hw
    // components to migrate from HSM targer to HSM parent)
    /* let mut hw_components_missing_in_user_request: HashMap<String, isize> = HashMap::new();
    for (_xname, hw_component_counter_hashmap) in &target_hsm_node_hw_component_count_vec {
        for (hw_component, quantity) in hw_component_counter_hashmap {
            if !user_defined_hw_component_vec.contains(&hw_component) {
                hw_components_missing_in_user_request
                    .entry(hw_component.to_string())
                    .and_modify(|old_quantity| *old_quantity -= *quantity as isize)
                    .or_insert(-(*quantity as isize));
            }
        }
    } */

    /* println!(
        "DEBUG - hw_components_missing_in_user_request: {:?}",
        hw_components_missing_in_user_request
    ); */

    // Merge hw components in target HSM missing in user request hw components with the user
    // request hw components - this are effectively the hw components we want to remove
    /* hw_components_to_migrate_from_target_hsm_to_parent_hsm
    .extend(hw_components_missing_in_user_request); */

    /* println!(
        "DEBUG - hw_components_to_migrate_from_target_hsm_to_parent_hsm: {:?}",
        hw_components_to_migrate_from_target_hsm_to_parent_hsm
    ); */

    /* println!(
        "Components to move from '{}' to '{}' --> {:?}",
        target_hsm_group_name,
        parent_hsm_group_name,
        hw_components_to_migrate_from_target_hsm_to_parent_hsm
    );

    println!(
        "Components to move from '{}' to '{}' --> {:?}",
        parent_hsm_group_name,
        target_hsm_group_name,
        hw_components_to_migrate_from_parent_hsm_to_target_hsm
    ); */

    /* let target_hsm_normalized_density_score_tuple_vec: Vec<(String, f32)> =
    calculate_hsm_hw_component_normalized_node_density_score_downscale(
        &target_hsm_node_hw_component_count_vec,
        &hw_components_to_migrate_from_target_hsm_to_parent_hsm,
        &user_defined_hw_component_count_hashmap,
        &target_hsm_hw_component_normalized_scores,
        &target_hsm_hw_component_count_hashmap,
        target_hsm_total_number_hw_components,
    ); */

    /* println!(
        "DEBUG - target_hsm_normalized_density_score_tuple_vec : {:#?}",
        target_hsm_normalized_density_score_tuple_vec
    ); */

    // *********************************************************************************************************
    // COLLECTIVE DATA TO HELP CALCULATING SCORES

    // Calculate hs component counters for target and parent hsm groups
    let mut target_parent_hsm_node_hw_component_count_vec = [
        target_hsm_node_hw_component_count_vec.clone(),
        parent_hsm_node_hw_component_count_vec.clone(),
    ]
    .concat();

    /* let target_parent_hsm_hw_component_summary_hashmap =
    get_hsm_hw_component_count_filtered_by_user_request(
        &user_defined_hw_component_vec,
        &node_hw_component_count_hashmap_target_parent_hsm_vec,
    ); */

    // Calculate hw component counters in HSM
    let target_parent_hsm_hw_component_count_hashmap: HashMap<String, usize> =
        calculate_hsm_hw_component_count(&target_parent_hsm_node_hw_component_count_vec);

    /* println!(
        "DEBUG - target_parent_hsm_hw_component_count_hashmap : {:?}",
        target_parent_hsm_hw_component_count_hashmap
    ); */

    // Calculate hw component counters in HSM filtered by user request
    let target_parent_hsm_hw_component_count_filtered_by_user_request_hashmap: HashMap<
        String,
        usize,
    > = get_hsm_hw_component_count_filtered_by_user_request(
        &user_defined_hw_component_vec,
        &target_parent_hsm_node_hw_component_count_vec,
    );

    println!(
        "HSM 'collective' hw component counters filtered by user request: {:?}",
        target_parent_hsm_hw_component_count_filtered_by_user_request_hashmap
    );

    // Calculate density scores for each node in HSM
    let target_parent_hsm_density_score_hashmap: HashMap<String, usize> =
        calculate_node_density_score(&target_parent_hsm_node_hw_component_count_vec);

    /* println!(
        "DEBUG - HSM 'colective' node's density score: {:?}",
        target_parent_hsm_density_score_hashmap
    ); */

    // Calculate total number of hw components in HSM
    let target_parent_hsm_total_number_hw_components: usize =
        calculate_hsm_total_number_hw_components(&target_parent_hsm_node_hw_component_count_vec);

    /* println!(
        "DEBUG - total_number_hw_components: {}",
        target_parent_hsm_total_number_hw_components
    ); */

    // Calculate nomarlized score for each hw component in HSM group
    let target_parent_hsm_hw_component_normalized_scores_hashmap =
        calculate_hsm_hw_component_normalized_density_score_from_hsm_node_hw_component_count_vec(
            &target_parent_hsm_node_hw_component_count_vec,
            target_parent_hsm_total_number_hw_components,
        );

    /* println!(
        "DEBUG - target_parent_hsm_hw_component_normalized_scores: {:?}",
        target_parent_hsm_hw_component_normalized_scores_hashmap
    ); */

    // Filter user request patterns with the hw components received from HSM hardware inventory
    /* println!(
        "DEBUG - user_defined_hw_component_count_hashmap: {:?}",
        user_defined_hw_component_count_hashmap
    );
    println!(
        "DEBUG - target_parent_hsm_hw_component_count_hashmap: {:?}",
        target_parent_hsm_hw_component_count_hashmap
    ); */
    user_defined_hw_component_count_hashmap.retain(|hw_component, _qty| {
        target_parent_hsm_hw_component_count_hashmap.contains_key(hw_component)
    });

    let (
        hw_components_to_migrate_from_target_hsm_to_parent_hsm,
        _hw_components_to_migrate_from_parent_hsm_to_target_hsm,
    ) = calculate_all_deltas(
        &user_defined_hw_component_count_hashmap,
        &target_parent_hsm_hw_component_count_filtered_by_user_request_hashmap,
    );

    /* // *********************************************************************************************************
    // FIND NODES TO MOVE FROM TARGET TO PARENT HSM GROUP

    println!(
        "\n----- TARGET HSM GROUP '{}' -----\n",
        target_hsm_group_name
    );

    println!(
        "Components to move from '{}' to '{}' --> {:?}",
        target_hsm_group_name,
        parent_hsm_group_name,
        hw_components_to_migrate_from_target_hsm_to_parent_hsm
    );

    // Calculate initial scores
    let target_hsm_score_tuple_vec = calculate_hsm_hw_component_normalized_node_density_score(
        &target_hsm_node_hw_component_count_vec,
        &hw_components_to_migrate_from_target_hsm_to_parent_hsm,
        &user_defined_hw_component_count_hashmap,
        &target_hsm_hw_component_normalized_scores,
        &target_hsm_hw_component_count_hashmap,
        target_hsm_total_number_hw_components,
    );

    /* println!(
        "DEBUG - target_hsm_group_hw_component_counter_vec:\n{:#?}",
        target_hsm_group_hw_component_counter_vec
    );
    println!(
        "DEBUG -hw_components_to_migrate_from_target_hsm_to_parent_hsm:\n{:#?}",
        hw_components_to_migrate_from_target_hsm_to_parent_hsm
    ); */

    // Migrate nodes
    let hw_component_counters_to_move_out_from_target_hsm = downscale_node_migration(
        &mut user_defined_hw_component_count_hashmap,
        &user_defined_hw_component_vec,
        &mut target_hsm_node_hw_component_count_vec,
        &target_hsm_density_score_hashmap,
        target_hsm_score_tuple_vec,
        hw_components_to_migrate_from_target_hsm_to_parent_hsm,
        &target_hsm_hw_component_normalized_scores,
        // &target_hsm_hw_component_count_hashmap,
    );

    // println!("DEBUG - hw_component_counters_to_move_out_from_target_hsm:\n{:?}", hw_component_counters_to_move_out_from_target_hsm); */

    /* // *********************************************************************************************************
    // Move nodes removed from Target HSM into Parent HSM group, calculate new summaries and calculate new deltas

    println!("\n----- UPDATE TARGET AND PARENT HSM GROUPS -----\n");

    parent_hsm_node_hw_component_count_vec = [
        parent_hsm_node_hw_component_count_vec,
        hw_component_counters_to_move_out_from_target_hsm.clone(),
    ]
    .concat();

    // Calculate hw component counters in HSM
    let parent_hsm_hw_component_count_hashmap_vec: HashMap<String, usize> =
        calculate_hsm_hw_component_count(&parent_hsm_node_hw_component_count_vec);

    println!(
        "DEBUG - parent_hsm_hw_component_count: {:?}",
        parent_hsm_hw_component_count_hashmap_vec
    );

    // Calculate hw component counters in HSM filtered by user request
    let parent_hsm_hw_component_count_filtered_by_user_request: HashMap<String, usize> =
        get_hsm_hw_component_count_filtered_by_user_request(
            &user_defined_hw_component_vec,
            &parent_hsm_node_hw_component_count_vec,
        );

    println!(
        "HSM '{}' hw component counters filtered by user request: {:?}",
        parent_hsm_group_name, parent_hsm_hw_component_count_filtered_by_user_request
    );

    // Calculate density scores for each node in HSM
    let parent_hsm_density_score_hashmap: HashMap<String, usize> =
        calculate_node_density_score(&parent_hsm_node_hw_component_count_vec);

    println!(
        "DEBUG - HSM '{}' node's density score: {:?}",
        parent_hsm_group_name, parent_hsm_density_score_hashmap
    );

    // Calculate total number of hw components in HSM
    let parent_hsm_total_number_hw_components: usize =
        calculate_hsm_total_number_hw_components(&mut parent_hsm_node_hw_component_count_vec);

    println!(
        "DEBUG - HSM '{}' --> total_number_hw_components: {}",
        parent_hsm_group_name, parent_hsm_total_number_hw_components
    );

    // Calculate nomarlized score for each hw component in HSM group
    let parent_hsm_hw_component_normalized_scores =
        calculate_hsm_hw_component_normalized_density_score_from_hsm_node_hw_component_count_vec(
            &parent_hsm_node_hw_component_count_vec,
            parent_hsm_total_number_hw_components,
        );

    println!(
        "DEBUG - parent_hsm_hw_component_normalized_scores: {:?}",
        parent_hsm_hw_component_normalized_scores
    ); */

    // *********************************************************************************************************
    // VALIDATION
    // Check collective HSM has enough capacity to process user request
    for (hw_component, qty_requested) in &user_defined_hw_component_count_hashmap {
        let qty_available = target_parent_hsm_hw_component_count_hashmap
            .get(hw_component)
            .unwrap();
        if qty_available < qty_requested {
            eprintln!("HSM 'collective' does not have enough resources to fulfill user request. User is requesting {} ({}) but only avaiable {}. Exit", hw_component, qty_requested, qty_available);
            std::process::exit(1);
        }
    }

    // *********************************************************************************************************
    // FIND NODES TO MOVE FROM PARENT TO TARGET HSM GROUP

    println!(
        "\n----- PARENT HSM GROUP '{}' -----\n",
        parent_hsm_group_name
    );

    let hw_components_to_migrate_from_parent_hsm_to_target_hsm: HashMap<String, isize> =
        user_defined_hw_component_count_hashmap
            .iter()
            .map(|(hw_inventory, count)| (hw_inventory.to_string(), -(*count as isize)))
            .collect();

    println!(
        "Components to move from '{}' to '{}' --> {:?}",
        parent_hsm_group_name,
        target_hsm_group_name,
        hw_components_to_migrate_from_parent_hsm_to_target_hsm
    );

    println!(
        "Components to move from '{}' to '{}' --> {:?}",
        parent_hsm_group_name, target_hsm_group_name, user_defined_hw_component_count_hashmap
    );

    // Calculate initial scores
    let target_parent_hsm_score_tuple_vec =
        calculate_hsm_hw_component_normalized_node_density_score_downscale(
            &target_parent_hsm_node_hw_component_count_vec,
            &hw_components_to_migrate_from_target_hsm_to_parent_hsm,
            &user_defined_hw_component_count_hashmap,
            &target_parent_hsm_hw_component_normalized_scores_hashmap,
            &target_parent_hsm_hw_component_count_hashmap,
        );

    /* println!(
        "DEBUG - user_defined_hw_component_counter_hashmap:\n{:#?}",
       user_defined_hw_component_counter_hashmap
    );
    println!(
        "DEBUG - user_defined_hw_component_vec:\n{:#?}",
       user_defined_hw_component_vec
    );
    println!(
        "DEBUG - parent_hsm_group_hw_component_counter_vec:\n{:#?}",
       parent_hsm_group_hw_component_counter_vec
    );
    println!(
        "DEBUG - parent_hsm_density_score_hashmap:\n{:#?}",
       parent_hsm_density_score_hashmap
    );
    println!(
        "DEBUG - parent_hsm_score_vec:\n{:#?}",
       parent_hsm_score_vec
    );
    println!(
        "DEBUG - hw_components_to_migrate_from_parent_hsm_to_target_hsm:\n{:#?}",
       hw_components_to_migrate_from_parent_hsm_to_target_hsm
    ); */

    // Migrate nodes
    let hw_component_counters_to_move_out_from_parent_hsm = upscale_node_migration(
        &user_defined_hw_component_count_hashmap,
        &user_defined_hw_component_vec,
        &mut target_parent_hsm_node_hw_component_count_vec,
        &target_parent_hsm_density_score_hashmap,
        target_parent_hsm_score_tuple_vec,
        hw_components_to_migrate_from_parent_hsm_to_target_hsm,
        &target_parent_hsm_hw_component_normalized_scores_hashmap,
    );

    // Sort target HSM group details
    let mut hsm_target_node_hw_component_count_vec =
        hw_component_counters_to_move_out_from_parent_hsm.clone();

    hsm_target_node_hw_component_count_vec.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    // Sort parent HSM group details
    target_parent_hsm_node_hw_component_count_vec.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    /* println!(
        "DEBUG - hw_component_counters_to_move_out_from_parent_hsm: {:?}",
        hw_component_counters_to_move_out_from_parent_hsm
    ); */

    println!("\n--------------------");
    println!("----- SOLUTION -----");
    println!("--------------------\n");

    println!(
        "DEBUG - SOL - target_parent_hsm_hw_component_count_hashmap: {:?}",
        target_parent_hsm_hw_component_count_hashmap
    );

    print_table_f32_score(
        &user_defined_hw_component_vec,
        &target_parent_hsm_node_hw_component_count_vec,
    );

    let target_hsm_hw_component_count_hashmap =
        calculate_hsm_hw_component_count(&hw_component_counters_to_move_out_from_parent_hsm);

    println!(
        "DEBUG - SOL - target_hsm_hw_component_count_hashmap: {:?}",
        target_hsm_hw_component_count_hashmap
    );

    print_table_f32_score(
        &user_defined_hw_component_vec,
        &hsm_target_node_hw_component_count_vec,
    );

    println!(
        "DEBUG - SOL - Target HSM '{}' members: {}",
        target_hsm_group_name,
        hsm_target_node_hw_component_count_vec
            .into_iter()
            .map(|node_hw_component| node_hw_component.0)
            .collect::<Vec<String>>()
            .join(", ")
    );

    println!(
        "DEBUG - SOL - user_defined_hw_component_count_hashmap: {:?}",
        user_defined_hw_component_count_hashmap
    );

    // *********************************************************************************************************
    // END MIGRATING NODES BETWEEN HSM GROUPS

    /* let mut new_target_hsm_members = hsm_group_target_members
        .into_iter()
        .filter(|node| {
            !hw_component_counters_to_move_out_from_target_hsm
                .iter()
                .any(|(node_aux, _)| node_aux.eq(node))
        })
        .collect::<Vec<String>>();

    let mut new_parent_hsm_members = [
        hsm_group_parent_members,
        hw_component_counters_to_move_out_from_target_hsm
            .into_iter()
            .map(|(node, _)| node)
            .collect(),
    ]
    .concat();

    new_parent_hsm_members = new_parent_hsm_members
        .into_iter()
        .filter(|node| {
            !hw_component_counters_to_move_out_from_parent_hsm
                .iter()
                .any(|(node_aux, _)| node_aux.contains(node))
        })
        .collect::<Vec<String>>();

    new_parent_hsm_members.sort();

    new_target_hsm_members = [
        new_target_hsm_members,
        hw_component_counters_to_move_out_from_parent_hsm
            .clone()
            .into_iter()
            .map(|(node, _)| node)
            .collect(),
    ]
    .concat();

    new_target_hsm_members.sort();

    println!(
        "HSM group '{}' members: {}",
        target_hsm_group_name,
        new_target_hsm_members.join(",")
    );

    println!(
        "HSM group '{}' members: {}",
        parent_hsm_group_name,
        new_parent_hsm_members.join(",")
    ); */
}

pub mod utils {
    use std::collections::HashMap;

    use comfy_table::Color;
    use serde_json::Value;

    /// Removes as much nodes as it can from the parent HSM group
    /// Returns a tuple with 2 vecs, the left one is the new parent HSM group while the left one is
    /// the one containing the nodes removed from the parent HSM
    pub fn upscale_node_migration(
        user_defined_hw_component_count_hashmap: &HashMap<String, usize>,
        user_defined_hw_component_vec: &Vec<String>,
        parent_hsm_node_hw_component_count_vec: &mut Vec<(String, HashMap<String, usize>)>,
        parent_hsm_density_score_hashmap: &HashMap<String, usize>,
        mut parent_hsm_score_tuple_vec: Vec<(String, f32)>,
        mut hw_components_to_migrate_from_parent_hsm_to_target_hsm: HashMap<String, isize>,
        parent_hsm_hw_component_normalized_scores_hashmap: &HashMap<String, f32>,
    ) -> Vec<(String, HashMap<String, usize>)> {
        if parent_hsm_score_tuple_vec.is_empty() {
            log::info!("No candidates to choose from");
            return Vec::new();
        }

        ////////////////////////////////
        // Initialize

        let mut nodes_migrated_from_parent_hsm: Vec<(String, HashMap<String, usize>)> = Vec::new();

        // Get best candidate
        let (mut best_candidate, mut best_candidate_counters) =
            get_best_candidate_to_upscale_migrate_f32_score(
                &mut parent_hsm_score_tuple_vec,
                parent_hsm_node_hw_component_count_vec,
            );

        // Check if we need to keep iterating
        let mut work_to_do =
            keep_iterating_upscale(&hw_components_to_migrate_from_parent_hsm_to_target_hsm);

        ////////////////////////////////
        // Itarate

        let mut iter = 0;

        while work_to_do {
            println!("-----------------------");
            println!("----- ITERATION {} -----", iter);
            println!("-----------------------\n");

            println!(
                "HW component counters requested by user: {:?}",
                user_defined_hw_component_count_hashmap
            );
            // Calculate HSM group hw component counters
            let parent_hsm_hw_component_count_hashmap =
                get_hsm_hw_component_count_filtered_by_user_request(
                    user_defined_hw_component_vec,
                    parent_hsm_node_hw_component_count_vec,
                );
            println!(
                "HSM group hw component counters: {:?}",
                parent_hsm_hw_component_count_hashmap
            );
            println!(
                "HW component counters yet to remove: {:?}",
                hw_components_to_migrate_from_parent_hsm_to_target_hsm
            );
            println!(
                "Best candidate is '{}' with score {} and hw component counters {:?}\n",
                best_candidate.0,
                parent_hsm_score_tuple_vec
                    .iter()
                    .find(|(node, _score)| node.eq(&best_candidate.0))
                    .unwrap()
                    .1,
                best_candidate_counters
            );

            // Print target hsm group hw configuration in table
            print_table_f32_score(
                user_defined_hw_component_vec,
                parent_hsm_node_hw_component_count_vec,
                parent_hsm_density_score_hashmap,
                &parent_hsm_score_tuple_vec,
            );

            ////////////////////////////////
            // Apply changes - Migrate from target to parent HSM

            // Add best candidate to parent HSM group
            nodes_migrated_from_parent_hsm
                .push((best_candidate.0.clone(), best_candidate_counters.clone()));

            // Remove best candidate from target HSM group
            parent_hsm_node_hw_component_count_vec.retain(|(node, _)| !node.eq(&best_candidate.0));

            if parent_hsm_node_hw_component_count_vec.is_empty() {
                break;
            }

            // Update hw component counters to migrate
            hw_components_to_migrate_from_parent_hsm_to_target_hsm =
                update_user_defined_hw_component_counters(
                    &hw_components_to_migrate_from_parent_hsm_to_target_hsm,
                    &best_candidate_counters,
                );

            // Calculate hw component counters in HSM
            /* let parent_hsm_hw_component_count_hashmap =
            calculate_hsm_hw_component_count(parent_hsm_node_hw_component_count_vec); */
            /* println!(
                "DEBUG - New HSM hw component counters: {:?}",
                parent_hsm_hw_component_count_hashmap
            ); */

            // Calculate total hw component counters in HSM
            /* let parent_hsm_total_number_hw_components: usize =
            calculate_hsm_total_number_hw_components(parent_hsm_node_hw_component_count_vec); */
            /* println!(
                "DEBUG - Total number hw components: {}",
                parent_hsm_total_number_hw_components
            ); */

            // Update scores
            parent_hsm_score_tuple_vec =
                calculate_hsm_hw_component_normalized_node_density_score_upscale(
                    parent_hsm_node_hw_component_count_vec,
                    &hw_components_to_migrate_from_parent_hsm_to_target_hsm,
                    // user_defined_hw_component_count_hashmap,
                    parent_hsm_hw_component_normalized_scores_hashmap,
                    // &parent_hsm_hw_component_count_hashmap,
                    // parent_hsm_total_number_hw_components,
                );

            // Remove best candidate from scores
            parent_hsm_score_tuple_vec.retain(|(node, _)| !node.eq(&best_candidate.0));

            // Get best candidate
            (best_candidate, best_candidate_counters) =
                get_best_candidate_to_upscale_migrate_f32_score(
                    &mut parent_hsm_score_tuple_vec,
                    parent_hsm_node_hw_component_count_vec,
                );

            // Check if we need to keep iterating
            work_to_do =
                keep_iterating_upscale(&hw_components_to_migrate_from_parent_hsm_to_target_hsm);

            iter += 1;
        }

        println!("\n------------------------");
        println!("----- FINAL RESULT -----");
        println!("------------------------\n");

        println!("No candidates found\n");

        // Print target hsm group hw configuration in table
        print_table_f32_score(
            user_defined_hw_component_vec,
            parent_hsm_node_hw_component_count_vec,
            parent_hsm_density_score_hashmap,
            &parent_hsm_score_tuple_vec,
        );

        nodes_migrated_from_parent_hsm
    }

    pub fn keep_iterating_downscale(
        hw_components_to_migrate_from_target_hsm_to_parent_hsm: &HashMap<String, isize>,
        best_candidate_counters: &HashMap<String, usize>,
    ) -> bool {
        let mut work_to_do = true;

        // Check best candidate has hw components to remove. Otherwise, this node should not be
        // considered a candidate
        // TODO: move this to the function that selects best candidate?
        if best_candidate_counters
            .keys()
            .all(|best_candidate_hw_component| {
                !hw_components_to_migrate_from_target_hsm_to_parent_hsm
                    .contains_key(best_candidate_hw_component)
            })
        {
            println!("Stop processing because none of the hw components in best candidate should be removed. Best candidate {:?}, hw components to remove {:?}", best_candidate_counters, hw_components_to_migrate_from_target_hsm_to_parent_hsm);
            work_to_do = false;
        }

        for (hw_component, quantity) in hw_components_to_migrate_from_target_hsm_to_parent_hsm {
            /* println!("\nDEBUG - hw_component: {}", hw_component);
            println!("DEBUG - quantity: {}", quantity);
            println!(
                "DEBUG - best_candidate_counters: {:?}\n",
                best_candidate_counters
            ); */
            if best_candidate_counters.get(hw_component).is_some()
                && quantity.unsigned_abs() < *best_candidate_counters.get(hw_component).unwrap()
            {
                println!("Stop processing because otherwise user will get less hw components ({}) than requested because best candidate has {} and we have {} left", hw_component, best_candidate_counters.get(hw_component).unwrap(), quantity.abs());
                work_to_do = false;
                break;
            }

            /* if quantity.abs() > 0
                && best_candidate_counters.get(hw_component).is_some()
                && best_candidate_counters.get(hw_component).unwrap() <= &(quantity.unsigned_abs())
            {
                work_to_do = true;
                break;
            } */
        }

        work_to_do
    }

    pub fn keep_iterating_upscale(
        hw_components_to_migrate_from_target_hsm_to_parent_hsm: &HashMap<String, isize>,
    ) -> bool {
        /* println!("DEBUG - inspecting if need to continue iterating ...");
        println!(
            "DEBUG - hw_components_to_migrate_from_target_hsm_to_parent_hsm: {:?}",
            hw_components_to_migrate_from_target_hsm_to_parent_hsm
        );
        println!("DEBUG - best_candidate_counters: {:?}", best_candidate_counters); */

        let mut work_to_do = false;

        for quantity in hw_components_to_migrate_from_target_hsm_to_parent_hsm.values() {
            if quantity.abs() > 0
            /* && best_candidate_counters.get(hw_component).is_some()
            && best_candidate_counters.get(hw_component).unwrap() >= &(quantity.unsigned_abs()) */
            {
                work_to_do = true;
                break;
            }
        }

        work_to_do
    }

    /// Removes as much nodes as it can from the target HSM group
    /// Returns a tuple with 2 vecs, the left one is the new target HSM group while the left one is
    /// the one containing the nodes removed from the target HSM
    pub fn downscale_node_migration(
        user_defined_hw_component_count_hashmap: &HashMap<String, usize>,
        user_defined_hw_component_vec: &Vec<String>,
        target_hsm_node_hw_component_count_vec: &mut Vec<(String, HashMap<String, usize>)>,
        target_hsm_node_density_score_hashmap: &HashMap<String, usize>,
        mut target_hsm_score_tuple_vec: Vec<(String, f32)>,
        mut hw_components_to_migrate_from_target_hsm_to_parent_hsm: HashMap<String, isize>,
        target_hsm_hw_component_normalized_scores_hashmap: &HashMap<String, f32>,
        // target_hsm_hw_component_count_hashmap: &HashMap<String, usize>,
    ) -> Vec<(String, HashMap<String, usize>)> {
        if target_hsm_score_tuple_vec.is_empty() {
            log::info!("No candidates to choose from");
            return Vec::new();
        }

        ////////////////////////////////
        // Initialize

        let mut nodes_migrated_from_target_hsm: Vec<(String, HashMap<String, usize>)> = Vec::new();

        // Get best candidate
        let (mut best_candidate, mut best_candidate_counters) =
            get_best_candidate_to_downscale_migrate_f32_score(
                &mut target_hsm_score_tuple_vec,
                target_hsm_node_hw_component_count_vec,
            );

        /* println!("DEBUG - best_candidate: {:?}", best_candidate);
        println!(
            "DEBUG - best_candidate_counters: {:?}",
            best_candidate_counters
        ); */

        // Check if we need to keep iterating
        let mut work_to_do = keep_iterating_downscale(
            &hw_components_to_migrate_from_target_hsm_to_parent_hsm,
            &best_candidate_counters,
        );

        ////////////////////////////////
        // Itarate

        let mut iter = 0;

        while work_to_do {
            println!("-----------------------");
            println!("----- ITERATION {} -----", iter);
            println!("-----------------------\n");

            println!(
                "HW component counters requested by user: {:?}",
                user_defined_hw_component_count_hashmap
            );
            // Calculate HSM group hw component counters
            let target_hsm_hw_component_filtered_by_user_request_count_hashmap =
                get_hsm_hw_component_count_filtered_by_user_request(
                    user_defined_hw_component_vec,
                    target_hsm_node_hw_component_count_vec,
                );
            println!(
                "HSM group hw component counters: {:?}",
                target_hsm_hw_component_filtered_by_user_request_count_hashmap
            );
            println!(
                "HW component counters yet to remove: {:?}",
                hw_components_to_migrate_from_target_hsm_to_parent_hsm
            );
            println!(
                "Best candidate is '{}' with score {} and hw component counters {:?}\n",
                best_candidate.0,
                target_hsm_score_tuple_vec
                    .iter()
                    .find(|(node, _score)| node.eq(&best_candidate.0))
                    .unwrap()
                    .1,
                best_candidate_counters
            );

            // Print target hsm group hw configuration in table
            print_table_f32_score(
                user_defined_hw_component_vec,
                target_hsm_node_hw_component_count_vec,
                target_hsm_node_density_score_hashmap,
                &target_hsm_score_tuple_vec,
            );

            ////////////////////////////////
            // Apply changes - Migrate from target to parent HSM

            // Add best candidate to parent HSM group
            nodes_migrated_from_target_hsm
                .push((best_candidate.0.clone(), best_candidate_counters.clone()));

            // Remove best candidate from target HSM grour
            target_hsm_node_hw_component_count_vec.retain(|(node, _)| !node.eq(&best_candidate.0));

            if target_hsm_node_hw_component_count_vec.is_empty() {
                break;
            }

            // Update hw component counters to migrate
            hw_components_to_migrate_from_target_hsm_to_parent_hsm =
                update_user_defined_hw_component_counters(
                    &hw_components_to_migrate_from_target_hsm_to_parent_hsm,
                    &best_candidate_counters,
                );

            // Calculate total number of hw components in hsm group
            /* println!(
                "DEBUG - ########### hsm hw components: {:?}",
                target_hsm_hw_component_count_vec
            ); */

            let target_hsm_hw_component_count_hashmap =
                calculate_hsm_hw_component_count(target_hsm_node_hw_component_count_vec);
            /* println!(
                "DEBUG - New HSM hw component counters: {:?}",
                target_hsm_hw_component_count_hashmap
            ); */

            /* let target_hsm_total_number_hw_components: usize =
            calculate_hsm_total_number_hw_components(target_hsm_node_hw_component_count_vec); */
            /* println!(
                "DEBUG - Total number hw components: {}",
                target_hsm_total_number_hw_components
            ); */

            // Update scores
            target_hsm_score_tuple_vec =
                calculate_hsm_hw_component_normalized_node_density_score_downscale(
                    target_hsm_node_hw_component_count_vec,
                    &hw_components_to_migrate_from_target_hsm_to_parent_hsm,
                    user_defined_hw_component_count_hashmap,
                    target_hsm_hw_component_normalized_scores_hashmap,
                    &target_hsm_hw_component_count_hashmap,
                );

            // Remove best candidate from scores
            target_hsm_score_tuple_vec.retain(|(node, _)| !node.eq(&best_candidate.0));

            // Get best candidate
            (best_candidate, best_candidate_counters) =
                get_best_candidate_to_downscale_migrate_f32_score(
                    &mut target_hsm_score_tuple_vec,
                    target_hsm_node_hw_component_count_vec,
                );

            // Check if we need to keep iterating
            work_to_do = keep_iterating_downscale(
                &hw_components_to_migrate_from_target_hsm_to_parent_hsm,
                &best_candidate_counters,
            );

            iter += 1;
        }

        println!("\n------------------------");
        println!("----- FINAL RESULT -----");
        println!("------------------------\n");

        println!("No candidates found\n");

        // Print target hsm group hw configuration in table
        print_table_f32_score(
            user_defined_hw_component_vec,
            target_hsm_node_hw_component_count_vec,
            target_hsm_node_density_score_hashmap,
            &target_hsm_score_tuple_vec,
        );

        nodes_migrated_from_target_hsm
    }

    pub fn update_user_defined_hw_component_counters(
        user_defined_hw_component_counter_hashmap: &HashMap<String, isize>,
        best_node_candidate_hashmap: &HashMap<String, usize>,
    ) -> HashMap<String, isize> {
        let mut new_user_defined_hw_component_counter_hashmap = HashMap::new();

        for (hw_component, quantity) in user_defined_hw_component_counter_hashmap {
            if best_node_candidate_hashmap.contains_key(hw_component) {
                let new_quantity = (*quantity)
                    + (*best_node_candidate_hashmap.get(hw_component).unwrap() as isize);

                if new_quantity <= 0 {
                    new_user_defined_hw_component_counter_hashmap
                        .insert(hw_component.to_string(), new_quantity);
                }
            } else {
                new_user_defined_hw_component_counter_hashmap
                    .insert(hw_component.clone(), *quantity);
            }
        }

        new_user_defined_hw_component_counter_hashmap
    }

    /* pub fn calculate_scores_scores(
        target_hsm_group_hw_component_counter_vec: &Vec<(String, HashMap<String, usize>)>,
        hw_components_to_migrate_from_target_hsm_to_parent_hsm: &HashMap<String, isize>,
    ) -> Vec<(String, isize)> {
        // Calculate HSM scores
        let target_hsm_score_vec: Vec<(String, isize)> = calculate_scores(
            target_hsm_group_hw_component_counter_vec,
            hw_components_to_migrate_from_target_hsm_to_parent_hsm,
        );

        target_hsm_score_vec
    } */

    pub fn get_best_candidate_to_downscale_migrate(
        target_hsm_score_vec: &mut [(String, isize)],
        target_hsm_hw_component_vec: &[(String, HashMap<String, usize>)],
    ) -> ((String, isize), HashMap<String, usize>) {
        target_hsm_score_vec.sort_by(|b, a| a.1.partial_cmp(&b.1).unwrap());

        // Get node with highest normalized score (best candidate)
        let highest_normalized_score: isize = *target_hsm_score_vec
            .iter()
            .map(|(_, normalized_score)| normalized_score)
            .max()
            .unwrap();

        let best_candidate: (String, isize) = target_hsm_score_vec
            .iter()
            .find(|(_, normalized_score)| *normalized_score == highest_normalized_score)
            .unwrap()
            .clone();

        let best_candidate_counters = &target_hsm_hw_component_vec
            .iter()
            .find(|(node, _)| node.eq(&best_candidate.0))
            .unwrap()
            .1;

        /* println!(
            "Best candidate is '{}' with a normalized score of {} with alphas {:?}",
            best_candidate.0, highest_normalized_score, best_candidate_counters
        ); */

        (best_candidate, best_candidate_counters.clone())
    }

    pub fn get_best_candidate_to_downscale_migrate_f32_score(
        target_hsm_score_vec: &mut [(String, f32)],
        target_hsm_hw_component_vec: &[(String, HashMap<String, usize>)],
    ) -> ((String, f32), HashMap<String, usize>) {
        target_hsm_score_vec.sort_by(|b, a| a.1.partial_cmp(&b.1).unwrap());

        // Get node with highest normalized score (best candidate)
        let best_candidate: (String, f32) = target_hsm_score_vec
            .iter_mut()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .unwrap()
            .clone();

        let best_candidate_counters = &target_hsm_hw_component_vec
            .iter()
            .find(|(node, _)| node.eq(&best_candidate.0))
            .unwrap()
            .1;

        /* println!(
            "Best candidate is '{}' with a normalized score of {} with alphas {:?}",
            best_candidate.0, highest_normalized_score, best_candidate_counters
        ); */

        (best_candidate, best_candidate_counters.clone())
    }

    pub fn get_best_candidate_to_upscale_migrate(
        parent_hsm_score_vec: &mut [(String, isize)],
        parent_hsm_hw_component_vec: &[(String, HashMap<String, usize>)],
    ) -> ((String, isize), HashMap<String, usize>) {
        parent_hsm_score_vec.sort_by(|b, a| a.1.partial_cmp(&b.1).unwrap());

        // Get node with highest normalized score (best candidate)
        let highest_normalized_score: isize = *parent_hsm_score_vec
            .iter()
            .map(|(_, normalized_score)| normalized_score)
            .min()
            .unwrap();

        let best_candidate: (String, isize) = parent_hsm_score_vec
            .iter()
            .find(|(_, normalized_score)| *normalized_score == highest_normalized_score)
            .unwrap()
            .clone();

        let best_candidate_counters = &parent_hsm_hw_component_vec
            .iter()
            .find(|(node, _)| node.eq(&best_candidate.0))
            .unwrap()
            .1;

        /* println!(
            "Best candidate is '{}' with a normalized score of {} with alphas {:?}",
            best_candidate.0, highest_normalized_score, best_candidate_counters
        ); */

        (best_candidate, best_candidate_counters.clone())
    }

    pub fn get_best_candidate_to_upscale_migrate_f32_score(
        parent_hsm_score_vec: &mut [(String, f32)],
        parent_hsm_hw_component_vec: &[(String, HashMap<String, usize>)],
    ) -> ((String, f32), HashMap<String, usize>) {
        parent_hsm_score_vec.sort_by(|b, a| a.1.partial_cmp(&b.1).unwrap());

        // Get node with highest normalized score (best candidate)
        let best_candidate: (String, f32) = parent_hsm_score_vec
            .iter_mut()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .unwrap()
            .clone();

        let best_candidate_counters = &parent_hsm_hw_component_vec
            .iter()
            .find(|(node, _)| node.eq(&best_candidate.0))
            .unwrap()
            .1;

        /* println!(
            "Best candidate is '{}' with a normalized score of {} with alphas {:?}",
            best_candidate.0, highest_normalized_score, best_candidate_counters
        ); */

        (best_candidate, best_candidate_counters.clone())
    }

    // Calculates node score based on hw component density
    pub fn calculate_node_density_score(
        node_hw_component_count_hashmap_target_hsm_vec: &Vec<(String, HashMap<String, usize>)>,
    ) -> HashMap<String, usize> {
        let mut target_hsm_density_score_hashmap: HashMap<String, usize> = HashMap::new();
        for target_hsm_group_hw_component in node_hw_component_count_hashmap_target_hsm_vec {
            let node_density_score = target_hsm_group_hw_component.1.values().sum();
            target_hsm_density_score_hashmap
                .insert(target_hsm_group_hw_component.clone().0, node_density_score);
        }

        target_hsm_density_score_hashmap
    }

    // Given a node hw conter, user request hw conter and HSM hw counters, calculate if it is save
    // to remove this node form the HSM and still be able to fullful the user request using the
    // rest of nodes in the HSM group. This method is used to validate if HSM group still can
    // fulfill user request after removing a node.
    pub fn can_node_be_removed_without_violating_user_request(
        node_hw_component_count: &HashMap<String, usize>,
        user_request_hw_components_count_hashmap: &HashMap<String, usize>,
        target_hsm_node_hw_components_count_hashmap: &HashMap<String, usize>,
    ) -> bool {
        /* println!(
            "DEBUG - user_request_hw_components_count_hashmap: {:?}",
            user_request_hw_components_count_hashmap
        ); */
        for (hw_component, requested_qty) in user_request_hw_components_count_hashmap {
            if let Some(node_hw_component_count) = node_hw_component_count.get(hw_component) {
                let target_hsm_hw_component_count: usize =
                    *target_hsm_node_hw_components_count_hashmap
                        .get(hw_component)
                        .unwrap();
                /* println!(
                    "DEBUG - User requested hw component {} Node qty {} HSM qty {}",
                    hw_component, node_hw_component_count, target_hsm_hw_component_count
                ); */

                return target_hsm_hw_component_count - node_hw_component_count >= *requested_qty;
            }
        }

        true
    }

    /// Calculates a normalized score for each hw component in HSM group based on component
    /// scarcity.
    pub fn calculate_hsm_hw_component_normalized_node_density_score_downscale(
        target_hsm_node_hw_component_count_hashmap_vec: &Vec<(String, HashMap<String, usize>)>,
        hw_components_to_migrate_from_one_hsm_to_another_hsm: &HashMap<String, isize>,
        hw_component_count_requested_by_user: &HashMap<String, usize>,
        target_hsm_hw_component_normalized_scores: &HashMap<String, f32>,
        target_hsm_hw_component_count_hashmap: &HashMap<String, usize>,
    ) -> Vec<(String, f32)> {
        /* println!(
            "DEBUG - node_hw_component_count_hashmap_target_hsm_vec: {:?}",
            target_hsm_node_hw_component_count_hashmap_vec
        );

        println!(
            "DEBUG - hw_components_to_migrate_from_one_hsm_to_another_hsm: {:?}",
            hw_components_to_migrate_from_one_hsm_to_another_hsm
        );

        println!(
            "DEBUG - hsm_hw_component_count: {:?}",
            hsm_hw_component_count
        ); */

        let mut target_hsm_density_score_hashmap: HashMap<String, f32> = HashMap::new();

        for (xname, node_hw_component_count) in target_hsm_node_hw_component_count_hashmap_vec {
            let mut node_hw_normalize_score = 0f32;

            /* println!(
                "DEBUG - #### NODE '{}' counters: {:?} ####",
                xname, node_hw_component_count
            ); */
            if !can_node_be_removed_without_violating_user_request(
                node_hw_component_count,
                hw_component_count_requested_by_user,
                target_hsm_hw_component_count_hashmap,
            ) {
                // cannot remove this hw component, otherwise we won't have enough
                // resources to fullfil user request
                // println!("DEBUG - cannot remove node '{}', otherwise we will violate user request, updating score to negative one", xname);
                node_hw_normalize_score = f32::MIN;
            } else {
                for hw_component in node_hw_component_count.keys() {
                    let hw_component_normalize_score: f32 =
                        if hw_components_to_migrate_from_one_hsm_to_another_hsm
                            .get(hw_component)
                            .is_some_and(|qty| qty.abs() > 0)
                        {
                            /* let hsm_hw_component_count_hashmap: HashMap<String, usize> =
                            calculate_hsm_hw_component_count(
                                target_hsm_node_hw_component_count_hashmap_vec,
                            ); */
                            // println!("DEBUG - hw component '{}' needs to go", hw_component);
                            // found hw component that needs to move out
                            /* let quantity_yet_to_go =
                            hw_components_to_migrate_from_one_hsm_to_another_hsm
                                .get(hw_component)
                                .unwrap()
                                .abs() as usize; */
                            // println!(
                            //     "DEBUG - hw component: {}, quantity node has: {}, quantity to go: {}",
                            //     hw_component, *qty as f32, quantity_yet_to_go
                            // );
                            if can_node_be_removed_without_violating_user_request(
                                node_hw_component_count,
                                hw_component_count_requested_by_user,
                                target_hsm_hw_component_count_hashmap,
                            ) {
                                // found hw component in hsm goup with enough quantity to remove
                                /* println!(
                                    "DEBUG - qty: {}, hsm_hw_component_count: {}",
                                    qty, hsm_hw_component_count
                                ); */
                                100_f32
                                    - (*target_hsm_hw_component_normalized_scores
                                        .get(hw_component)
                                        .unwrap())
                            } else {
                                // cannot remove this hw component, otherwise we won't have enough
                                // resources to fullfil user request
                                // println!("DEBUG - cannot remove hw component '{}', otherwise we will violate user request, updating score to negative one", hw_component);
                                -(100_f32
                                    - (*target_hsm_hw_component_normalized_scores
                                        .get(hw_component)
                                        .unwrap()))
                            }
                        } else {
                            // found a hw component the user is not asking for
                            // println!(
                            //     "DEBUG - hw component: {} to look for in {:?}",
                            //     hw_component, target_hsm_hw_component_normalized_scores
                            // );
                            -(100_f32
                                - (*target_hsm_hw_component_normalized_scores
                                    .get(hw_component)
                                    .unwrap()))
                        };

                    /* println!(
                        "DEBUG - node: {}, hw component: {}, normalized score: {}",
                        xname, hw_component, hw_component_normalize_score
                    ); */
                    node_hw_normalize_score += hw_component_normalize_score;
                }
            }

            target_hsm_density_score_hashmap.insert(xname.to_string(), node_hw_normalize_score);
        }

        let target_hsm_normalized_density_score_tuple_vec: Vec<(String, f32)> =
            target_hsm_density_score_hashmap
                .into_iter()
                .map(|(k, v)| (k, v))
                .collect();

        target_hsm_normalized_density_score_tuple_vec
    }

    pub fn calculate_hsm_hw_component_normalized_node_density_score_upscale(
        hsm_node_hw_component_count_hashmap_vec: &Vec<(String, HashMap<String, usize>)>,
        hw_components_to_migrate_from_one_hsm_to_another_hsm: &HashMap<String, isize>,
        // hw_component_count_requested_by_user: &HashMap<String, usize>,
        hsm_hw_component_normalized_scores: &HashMap<String, f32>,
        // hsm_hw_component_count_hashmap: &HashMap<String, usize>,
        // hsm_hw_component_count: usize,
    ) -> Vec<(String, f32)> {
        /* println!(
            "DEBUG - node_hw_component_count_hashmap_target_hsm_vec: {:?}",
            hsm_node_hw_component_count_hashmap_vec
        ); */

        /* println!(
            "DEBUG - hw_components_to_migrate_from_one_hsm_to_another_hsm: {:?}",
            hw_components_to_migrate_from_one_hsm_to_another_hsm
        ); */

        /* println!(
            "DEBUG - hsm_hw_component_count: {:?}",
            hsm_hw_component_count
        ); */

        let mut hsm_density_score_hashmap: HashMap<String, f32> = HashMap::new();

        for (xname, node_hw_component_count) in hsm_node_hw_component_count_hashmap_vec {
            let mut node_hw_normalize_score = 0f32;

            /* println!(
                "DEBUG - #### NODE '{}' counters: {:?} ####",
                xname, node_hw_component_count
            ); */
            /* if !can_node_be_removed_without_violating_user_request(
                node_hw_component_count,
                hw_component_count_requested_by_user,
                hsm_hw_component_count_hashmap,
            ) {
                // cannot remove this hw component, otherwise we won't have enough
                // resources to fullfil user request
                // println!("DEBUG - cannot remove node '{}', otherwise we will violate user request, updating score to negative one", xname);
                // node_hw_normalize_score = f32::MIN;
            } else { */
            for hw_component in node_hw_component_count.keys() {
                let hw_component_normalize_score: f32 =
                    if hw_components_to_migrate_from_one_hsm_to_another_hsm
                        .get(hw_component)
                        .is_some_and(|qty| qty.abs() > 0)
                    {
                        // found hw component that needs to move out
                        100_f32
                            - (*hsm_hw_component_normalized_scores
                                .get(hw_component)
                                .unwrap())
                    } else {
                        // found a hw component the user is not asking for
                        /* println!(
                            "DEBUG - hw component: {} to look for in {:?}",
                            hw_component, hsm_hw_component_normalized_scores
                        ); */
                        -(100_f32
                            - (*hsm_hw_component_normalized_scores
                                .get(hw_component)
                                .unwrap()))
                    };

                /* println!(
                    "DEBUG - node: {}, hw component: {}, normalized score: {}",
                    xname, hw_component, hw_component_normalize_score
                ); */
                node_hw_normalize_score += hw_component_normalize_score;
            }
            // }

            hsm_density_score_hashmap.insert(xname.to_string(), node_hw_normalize_score);
        }

        let target_hsm_normalized_density_score_tuple_vec: Vec<(String, f32)> =
            hsm_density_score_hashmap
                .into_iter()
                .map(|(k, v)| (k, v))
                .collect();

        target_hsm_normalized_density_score_tuple_vec
    }

    pub fn calculate_hsm_hw_component_node_scores(
        hsm_group_hw_component_counter_vec: &Vec<(String, HashMap<String, usize>)>,
        hw_components_to_migrate_from_one_hsm_to_another_hsm: &HashMap<String, isize>,
    ) -> Vec<(String, isize)> {
        let mut target_hsm_score_vec: Vec<(String, isize)> = Vec::new();

        for (node, hw_components_count_hashmap) in hsm_group_hw_component_counter_vec {
            // println!("# Processing node: ({},{:?}", node, hw_components);

            // Calculate node's score
            let mut score = 0;
            for (hw_component, quantity) in hw_components_count_hashmap {
                // IMPORTANT TO LEAVE THIS AS IT IS, WE
                // WANT TO ITERATE THROUGH ALL HW
                // COMPONENTS IN THE NODE TO CALCULATE A
                // SCORE THAT REFLECTS PENALIZATION WHEN
                // THE COMPONENT IN THE NODE IS NOT
                // RELATED TO THE COMPONENTS REQUESTED BY
                // THE USER OR WHEN SELECING THIS NODE AS A CANDIDATE WOULD ACTUALLY IMPACT THE
                // NUMBER OF COMPONENTS THE USERS REQUESTS NEGATIVELY
                // NOTE: We are only seeing the node's
                // components related to what the user is
                // requesting... maybe we should get a
                // list of all relevant components
                // (processors and accelerators????) so we
                // can get a better idea of the node and
                // increase the penalization in the
                // score????
                let component_delta = if let Some(total_number_of_hw_components_to_remove) =
                    hw_components_to_migrate_from_one_hsm_to_another_hsm.get(hw_component)
                {
                    // hw_component is in user request
                    total_number_of_hw_components_to_remove
                } else {
                    // hw_component is not in user request
                    &0
                };

                // let component_delta = hw_components_to_migrate_from_one_hsm_to_another_hsm.get(hw_component).unwrap_or(&(quantity.to_owned() as isize));
                // .get(hw_component)
                // .unwrap_or(&0)
                // .clone();

                let component_score = if component_delta.unsigned_abs() >= *quantity {
                    // This hw component is in user request and is good to be a candidate
                    *quantity as isize
                    /* println!(
                        "hw component {} --> current score {} + node component score {} ==> new score {}",
                        hw_component, score, node_component_score, (score + component_score)
                    ); */
                } else {
                    // hw component either is not requested by user or can't be candidate
                    // (othwerwise user will receive less hw components than requested)

                    /* println!(
                        "delta ({}) < node component quantity ({}) --> node component score = 0",
                        component_delta, quantity
                    ); */
                    if hw_components_to_migrate_from_one_hsm_to_another_hsm
                        .iter()
                        .any(|(hw_component_requested_by_user, _quantity)| {
                            hw_component.contains(hw_component_requested_by_user)
                        })
                    {
                        // This hw component type is in user
                        // pattern but selecting this node as a candidate means the user would receive
                        // less number of components that it initially requested

                        -(*quantity as isize)
                    } else {
                        // This hw component type is not in user pattern, therefore, its quantity
                        // is going to count as a penalization to get the node evicted/migrated
                        // from the HSM group

                        *quantity as isize // We may want to add a penalization here...
                    }
                };

                /* println!(
                    "component {}, delta {}, component component_score {}",
                    hw_component, component_delta, component_score
                ); */

                score += component_score;
            }

            target_hsm_score_vec.push((node.to_string(), score));
        }

        target_hsm_score_vec
    }

    pub fn calculate_all_deltas(
        user_defined_hw_component_counter_hashmap: &HashMap<String, usize>,
        hsm_hw_component_summary_hashmap: &HashMap<String, usize>,
    ) -> (HashMap<String, isize>, HashMap<String, isize>) {
        /* println!(
            "DEBUG -- user_defined_hw_component_counter_hashmap: {:?}",
            user_defined_hw_component_counter_hashmap
        );
        println!(
            "DEBUG - hsm_hw_component_count_hashmap: {:?}",
            hsm_hw_component_count_hashmap
        ); */

        let mut hw_components_to_migrate_from_target_hsm_to_parent_hsm: HashMap<String, isize> =
            HashMap::new();

        let mut hw_components_to_migrate_from_parent_hsm_to_target_hsm: HashMap<String, isize> =
            HashMap::new();

        for (user_defined_hw_component, new_quantity) in user_defined_hw_component_counter_hashmap {
            let quantity_from = hsm_hw_component_summary_hashmap
                .get(user_defined_hw_component)
                .unwrap();

            let delta = (*new_quantity as isize) - (*quantity_from as isize);

            /* println!(
                "DEBUG - hw component {} : user request qty {} target hsm qty {} --> delta is {}",
                user_defined_hw_component, new_quantity, quantity_from, delta
            ); */

            /* if delta > 0 {
                // Migrate nodes from parent to target HSM group
                hw_components_to_migrate_from_parent_hsm_to_target_hsm
                    .insert(user_defined_hw_component.to_string(), -delta);
            } else if delta < 0 {
                // Migrate nodes from target to parent HSM group
                hw_components_to_migrate_from_target_hsm_to_parent_hsm
                    .insert(user_defined_hw_component.to_string(), delta);
            } else {
                // No change
            } */

            match delta as i32 {
                1.. =>
                // delta > 0 -> Migrate nodes from parent to target HSM group
                {
                    hw_components_to_migrate_from_parent_hsm_to_target_hsm
                        .insert(user_defined_hw_component.to_string(), -delta);
                }
                ..=-1 =>
                // delta < 0 -> Migrate nodes from target to parent HSM group
                {
                    hw_components_to_migrate_from_target_hsm_to_parent_hsm
                        .insert(user_defined_hw_component.to_string(), delta);
                }
                0 =>
                    // delta == 0 -> Do nothing
                    {}
            }
        }

        (
            hw_components_to_migrate_from_target_hsm_to_parent_hsm,
            hw_components_to_migrate_from_parent_hsm_to_target_hsm,
        )
    }

    /// Returns a triple like (<xname>, <list of hw components>, <list of memory capacity>)
    /// Note: list of hw components can be either the hw componentn pattern provided by user or the
    /// description from the HSM API
    pub async fn get_node_hw_component_count(
        shasta_token: String,
        shasta_base_url: String,
        shasta_root_cert: Vec<u8>,
        hsm_member: &str,
        user_defined_hw_profile_vec: Vec<String>,
    ) -> (String, Vec<String>, Vec<u64>) {
        let node_hw_inventory_value =
            mesa::hsm::hw_inventory::shasta::http_client::get_hw_inventory(
                &shasta_token,
                &shasta_base_url,
                &shasta_root_cert,
                hsm_member,
            )
            .await
            .unwrap();

        let node_hw_profile = get_node_hw_properties_from_value(
            &node_hw_inventory_value,
            user_defined_hw_profile_vec.clone(),
        );

        (hsm_member.to_string(), node_hw_profile.0, node_hw_profile.1)
    }

    // Calculate/groups hw component counters filtered by user request
    pub fn get_hsm_hw_component_count_filtered_by_user_request(
        user_defined_hw_component_vec: &Vec<String>,
        target_hsm_group_hw_component_vec: &Vec<(String, HashMap<String, usize>)>,
    ) -> HashMap<String, usize> {
        let mut hsm_hw_component_count_hashmap = HashMap::new();

        for x in user_defined_hw_component_vec {
            let mut hsm_hw_component_count = 0;
            for y in target_hsm_group_hw_component_vec {
                hsm_hw_component_count += y.1.get(x).unwrap_or(&0);
            }
            hsm_hw_component_count_hashmap.insert(x.to_string(), hsm_hw_component_count);
        }

        hsm_hw_component_count_hashmap
    }

    // Calculate/groups hw component counters
    pub fn calculate_hsm_hw_component_count(
        target_hsm_group_node_hw_component_vec: &Vec<(String, HashMap<String, usize>)>,
    ) -> HashMap<String, usize> {
        let mut hsm_hw_component_count_hashmap = HashMap::new();

        for (_xname, node_hw_component_count_hashmap) in target_hsm_group_node_hw_component_vec {
            for (hw_component, &qty) in node_hw_component_count_hashmap {
                hsm_hw_component_count_hashmap
                    .entry(hw_component.to_string())
                    .and_modify(|qty_aux| *qty_aux += qty)
                    .or_insert(qty);
            }
        }

        hsm_hw_component_count_hashmap
    }

    // Given a list of tuples (xname, list of hw components qty hasmap), this function will return
    // the list of hw components wih their quantity normalized in within the hsm group
    pub fn calculate_hsm_hw_component_normalized_density_score_from_hsm_node_hw_component_count_vec(
        hsm_node_hw_component_count_vec: &Vec<(String, HashMap<String, usize>)>,
        total_number_hw_components: usize,
    ) -> HashMap<String, f32> {
        let mut hsm_hw_component_count_hashmap: HashMap<String, usize> = HashMap::new();

        for (_xname, node_hw_component_count_hashmap) in hsm_node_hw_component_count_vec {
            for (hw_component, qty) in node_hw_component_count_hashmap {
                hsm_hw_component_count_hashmap
                    .entry(hw_component.to_string())
                    .and_modify(|qty_aux| *qty_aux += qty)
                    .or_insert(*qty);
            }
        }

        calculate_hsm_hw_component_normalized_density_score_from_hsm_hw_component_count_hashmap(
            hsm_hw_component_count_hashmap,
            total_number_hw_components,
        )
    }

    // Given the list of hw components qty in a node, this function will return the list of hw
    // components with their quantity normalized within the node
    pub fn calculate_hsm_hw_component_normalized_density_score_from_hsm_hw_component_count_hashmap(
        hsm_hw_component_count_hashmap: HashMap<String, usize>,
        total_number_hw_components: usize,
    ) -> HashMap<String, f32> {
        /* let total_number_hw_components = hsm_hw_component_count_hashmap
        .iter()
        .fold(0, |acc, (_hw_component, qty)| acc + qty); */

        /* let mut hsm_hw_component_normalized_score_hashmap: HashMap<String, f32> = HashMap::new();

        for (hw_component, qty) in hsm_hw_component_count_hashmap {
            hsm_hw_component_normalized_score_hashmap
                .entry(hw_component.to_string())
                .and_modify(|qty_aux| {
                    *qty_aux = *qty_aux as f32 / total_number_hw_components as f32
                })
                .or_insert(qty as f32 / total_number_hw_components as f32);
        }

        hsm_hw_component_normalized_score_hashmap */

        hsm_hw_component_count_hashmap
            .iter()
            .map(|(hw_component, qty)| {
                (
                    hw_component.to_string(),
                    (*qty * 100) as f32 / total_number_hw_components as f32,
                )
            })
            .collect()
    }

    /// Returns the properties in hw_property_list found in the node_hw_inventory_value which is
    /// HSM hardware inventory API json response
    pub fn get_node_hw_properties_from_value(
        node_hw_inventory_value: &Value,
        hw_component_pattern_list: Vec<String>,
    ) -> (Vec<String>, Vec<u64>) {
        let processor_vec = mesa::hsm::hw_inventory::shasta::utils::get_list_processor_model_from_hw_inventory_value(
            node_hw_inventory_value,
        )
        .unwrap_or_default();

        let accelerator_vec = mesa::hsm::hw_inventory::shasta::utils::get_list_accelerator_model_from_hw_inventory_value(
            node_hw_inventory_value,
        )
        .unwrap_or_default();

        /* let hsn_nic_vec =
        hsm::utils::get_list_hsn_nics_model_from_hw_inventory_value(node_hw_inventory_value)
            .unwrap_or_default(); */

        let processor_and_accelerator = [processor_vec, accelerator_vec].concat();

        let processor_and_accelerator_lowercase = processor_and_accelerator
            .iter()
            .map(|hw_component| hw_component.to_lowercase());

        /* let mut node_hw_component_pattern_vec = Vec::new();

        for actual_hw_component_pattern in processor_and_accelerator_lowercase {
            for hw_component_pattern in &hw_component_pattern_list {
                if actual_hw_component_pattern.contains(hw_component_pattern) {
                    node_hw_component_pattern_vec.push(hw_component_pattern.to_string());
                } else {
                    node_hw_component_pattern_vec.push(actual_hw_component_pattern.clone());
                }
            }
        }

        node_hw_component_pattern_vec */

        let mut node_hw_component_pattern_vec = Vec::new();

        for actual_hw_component_pattern in processor_and_accelerator_lowercase {
            if let Some(hw_component_pattern) = hw_component_pattern_list
                .iter()
                .find(|&hw_component| actual_hw_component_pattern.contains(hw_component))
            {
                node_hw_component_pattern_vec.push(hw_component_pattern.to_string());
            } else {
                node_hw_component_pattern_vec.push(actual_hw_component_pattern);
            }
        }

        let memory_vec = mesa::hsm::hw_inventory::shasta::utils::get_list_memory_capacity_from_hw_inventory_value(
            node_hw_inventory_value,
        )
        .unwrap_or_default();

        (node_hw_component_pattern_vec, memory_vec)
    }

    /// Calculates greatest common factor or lowest common multiple
    pub fn calculate_lcm(numbers: &Vec<u64>) -> u64 {
        let mut lcm = u64::MAX;
        for number in numbers {
            let factors = (1..number + 1)
                .filter(|&x| number % x == 0)
                .collect::<Vec<u64>>();

            println!("Prime factors for {} --> {:?}", number, factors);

            if factors.last().is_some() && factors.last().unwrap() < &lcm {
                lcm = *factors.last().unwrap();
            }
        }

        lcm
    }

    pub fn print_table(
        user_defined_hw_componet_vec: &[String],
        hsm_hw_pattern_vec: &[(String, HashMap<String, usize>)],
        hsm_density_score_hashmap: &HashMap<String, usize>,
        hsm_score_vec: &[(String, isize)],
    ) {
        /* println!("DEBUG - hsm_hw_pattern_vec:\n{:?}", hsm_hw_pattern_vec);
        println!(
            "DEBUG - hsm_density_score_hashmap:\n{:?}",
            hsm_density_score_hashmap
        ); */

        let hsm_hw_component_vec: Vec<String> = hsm_hw_pattern_vec
            .iter()
            .flat_map(|(_xname, node_pattern_hashmap)| node_pattern_hashmap.keys().cloned())
            .collect();

        let mut all_hw_component_vec =
            [hsm_hw_component_vec, user_defined_hw_componet_vec.to_vec()].concat();

        all_hw_component_vec.sort();
        all_hw_component_vec.dedup();

        // println!("DEBUG - all_hw_component_vec : {:?}", all_hw_component_vec);

        let mut table = comfy_table::Table::new();

        table.set_header(
            [
                vec!["Node".to_string()],
                all_hw_component_vec.clone(),
                vec!["Density Score".to_string()],
                vec!["Score".to_string()],
            ]
            .concat(),
        );

        for (xname, node_pattern_hashmap) in hsm_hw_pattern_vec {
            // println!("node_pattern_hashmap: {:?}", node_pattern_hashmap);

            let mut row: Vec<comfy_table::Cell> = Vec::new();
            // Node xname table cell
            row.push(
                comfy_table::Cell::new(xname.clone())
                    .set_alignment(comfy_table::CellAlignment::Center),
            );
            // User hw components table cell
            for hw_component in &all_hw_component_vec {
                if user_defined_hw_componet_vec.contains(hw_component)
                    && node_pattern_hashmap.contains_key(hw_component)
                {
                    let counter = node_pattern_hashmap.get(hw_component).unwrap();
                    row.push(
                        comfy_table::Cell::new(format!("✅ ({})", counter,))
                            .fg(Color::Green)
                            .set_alignment(comfy_table::CellAlignment::Center),
                    );
                } else if node_pattern_hashmap.contains_key(hw_component) {
                    let counter = node_pattern_hashmap.get(hw_component).unwrap();
                    row.push(
                        comfy_table::Cell::new(format!("\u{26A0} ({})", counter))
                            .fg(Color::Yellow)
                            .set_alignment(comfy_table::CellAlignment::Center),
                    );
                } else {
                    // node does not contain hardware but it was requested by the user
                    row.push(
                        comfy_table::Cell::new("❌".to_string())
                            .set_alignment(comfy_table::CellAlignment::Center),
                    );
                }
            }
            /* for user_defined_hw_component in user_defined_hw_componet_vec {
                if node_pattern_hashmap.contains_key(user_defined_hw_component) {
                    let counter = node_pattern_hashmap.get(user_defined_hw_component).unwrap();
                    row.push(
                        comfy_table::Cell::new(format!("✅ ({})", counter,))
                            .fg(Color::Green)
                            .set_alignment(comfy_table::CellAlignment::Center),
                    );
                } else {
                    row.push(
                        comfy_table::Cell::new("❌".to_string())
                            .set_alignment(comfy_table::CellAlignment::Center),
                    );
                }
            } */
            // Node density score table cell
            row.push(
                comfy_table::Cell::new(hsm_density_score_hashmap.get(xname).unwrap())
                    .set_alignment(comfy_table::CellAlignment::Center),
            );
            // Node score table cell
            let node_score = hsm_score_vec
                .iter()
                .find(|(node_name, _)| node_name.eq(xname))
                .unwrap()
                .1;
            let node_score_table_cell = if node_score <= 0 {
                comfy_table::Cell::new(node_score)
                    .set_alignment(comfy_table::CellAlignment::Center)
                    .fg(Color::Red)
            } else {
                comfy_table::Cell::new(node_score)
                    .set_alignment(comfy_table::CellAlignment::Center)
                    .fg(Color::Green)
            };
            row.push(node_score_table_cell);
            table.add_row(row);
        }

        println!("{table}\n");
    }

    pub fn print_table_f32_score(
        user_defined_hw_componet_vec: &[String],
        hsm_hw_pattern_vec: &[(String, HashMap<String, usize>)],
        hsm_density_score_hashmap: &HashMap<String, usize>,
        hsm_score_vec: &[(String, f32)],
    ) {
        /* println!("DEBUG - hsm_hw_pattern_vec:\n{:?}", hsm_hw_pattern_vec);
        println!(
            "DEBUG - hsm_density_score_hashmap:\n{:?}",
            hsm_density_score_hashmap
        ); */

        let hsm_hw_component_vec: Vec<String> = hsm_hw_pattern_vec
            .iter()
            .flat_map(|(_xname, node_pattern_hashmap)| node_pattern_hashmap.keys().cloned())
            .collect();

        let mut all_hw_component_vec =
            [hsm_hw_component_vec, user_defined_hw_componet_vec.to_vec()].concat();

        all_hw_component_vec.sort();
        all_hw_component_vec.dedup();

        // println!("DEBUG - all_hw_component_vec : {:?}", all_hw_component_vec);

        let mut table = comfy_table::Table::new();

        table.set_header(
            [
                vec!["Node".to_string()],
                all_hw_component_vec.clone(),
                vec!["Density Score".to_string()],
                vec!["Score".to_string()],
            ]
            .concat(),
        );

        for (xname, node_pattern_hashmap) in hsm_hw_pattern_vec {
            // println!("node_pattern_hashmap: {:?}", node_pattern_hashmap);

            let mut row: Vec<comfy_table::Cell> = Vec::new();
            // Node xname table cell
            row.push(
                comfy_table::Cell::new(xname.clone())
                    .set_alignment(comfy_table::CellAlignment::Center),
            );
            // User hw components table cell
            for hw_component in &all_hw_component_vec {
                if user_defined_hw_componet_vec.contains(hw_component)
                    && node_pattern_hashmap.contains_key(hw_component)
                {
                    let counter = node_pattern_hashmap.get(hw_component).unwrap();
                    row.push(
                        comfy_table::Cell::new(format!("✅ ({})", counter,))
                            .fg(Color::Green)
                            .set_alignment(comfy_table::CellAlignment::Center),
                    );
                } else if node_pattern_hashmap.contains_key(hw_component) {
                    let counter = node_pattern_hashmap.get(hw_component).unwrap();
                    row.push(
                        comfy_table::Cell::new(format!("⚠️ ({})", counter)) // NOTE: emojis
                            // can also be printed using unicode like \u{26A0}
                            .fg(Color::Yellow)
                            .set_alignment(comfy_table::CellAlignment::Center),
                    );
                } else {
                    // node does not contain hardware but it was requested by the user
                    row.push(
                        comfy_table::Cell::new("❌".to_string())
                            .set_alignment(comfy_table::CellAlignment::Center),
                    );
                }
            }
            /* for user_defined_hw_component in user_defined_hw_componet_vec {
                if node_pattern_hashmap.contains_key(user_defined_hw_component) {
                    let counter = node_pattern_hashmap.get(user_defined_hw_component).unwrap();
                    row.push(
                        comfy_table::Cell::new(format!("✅ ({})", counter,))
                            .fg(Color::Green)
                            .set_alignment(comfy_table::CellAlignment::Center),
                    );
                } else {
                    row.push(
                        comfy_table::Cell::new("❌".to_string())
                            .set_alignment(comfy_table::CellAlignment::Center),
                    );
                }
            } */
            // Node density score table cell
            row.push(
                comfy_table::Cell::new(hsm_density_score_hashmap.get(xname).unwrap_or(&0))
                    .set_alignment(comfy_table::CellAlignment::Center),
            );
            // Node score table cell
            let node_score = hsm_score_vec
                .iter()
                .find(|(node_name, _)| node_name.eq(xname))
                .unwrap_or(&(xname.to_string(), 0f32))
                .1;
            let node_score_table_cell = if node_score <= 0f32 {
                comfy_table::Cell::new(node_score)
                    .set_alignment(comfy_table::CellAlignment::Center)
                    .fg(Color::Red)
            } else {
                comfy_table::Cell::new(node_score)
                    .set_alignment(comfy_table::CellAlignment::Center)
                    .fg(Color::Green)
            };
            row.push(node_score_table_cell);
            table.add_row(row);
        }

        println!("{table}\n");
    }

    pub fn calculate_hsm_total_number_hw_components(
        target_hsm_hw_component_count_vec: &[(String, HashMap<String, usize>)],
    ) -> usize {
        target_hsm_hw_component_count_vec
            .iter()
            .flat_map(|(_node, hw_component_hashmap)| hw_component_hashmap.values())
            .sum()
    }
}

#[tokio::test]
pub async fn test_memory_capacity() {
    // XDG Base Directory Specification
    let project_dirs = directories::ProjectDirs::from(
        "local", /*qualifier*/
        "cscs",  /*organization*/
        "manta", /*application*/
    );

    // DEBUG
    let hsm_name = "zinal";

    let mut path_to_manta_configuration_file =
        std::path::PathBuf::from(project_dirs.unwrap().config_dir());

    path_to_manta_configuration_file.push("config.toml"); // ~/.config/manta/config is the file

    log::info!(
        "Reading manta configuration from {}",
        &path_to_manta_configuration_file.to_string_lossy()
    );

    let settings = crate::common::config_ops::get_configuration();

    let site_name = settings.get_string("site").unwrap();
    let site_detail_hashmap = settings.get_table("sites").unwrap();
    let site_detail_value = site_detail_hashmap
        .get(&site_name)
        .unwrap()
        .clone()
        .into_table()
        .unwrap();

    let shasta_base_url = site_detail_value
        .get("shasta_base_url")
        .unwrap()
        .to_string();

    let keycloak_base_url = site_detail_value
        .get("keycloak_base_url")
        .unwrap()
        .to_string();

    if let Some(socks_proxy) = site_detail_value.get("socks5_proxy") {
        std::env::set_var("SOCKS5", socks_proxy.to_string());
    }

    let shasta_root_cert = crate::common::config_ops::get_csm_root_cert_content(&site_name);

    let shasta_token = mesa::common::authentication::get_api_token(
        &shasta_base_url,
        &shasta_root_cert,
        &keycloak_base_url,
    )
    .await
    .unwrap();

    let hsm_group_vec = mesa::hsm::group::shasta::http_client::get_all(
        &shasta_token,
        &shasta_base_url,
        &shasta_root_cert,
    )
    .await
    .unwrap();
    /* let hsm_group_vec = hsm::http_client::get_hsm_group_vec(
        &shasta_token,
        &shasta_base_url,
        &shasta_root_cert,
        Some(&hsm_name.to_string()),
    )
    .await
    .unwrap(); */

    let mut node_hsm_groups_hw_inventory_map: HashMap<&str, (Vec<&str>, Vec<String>, Vec<u64>)> =
        HashMap::new();

    let new_vec = Vec::new();

    for hsm_group in &hsm_group_vec {
        let hsm_group_name = hsm_group["label"].as_str().unwrap();
        let hsm_member_vec: Vec<&str> = hsm_group["members"]["ids"]
            .as_array()
            .unwrap_or(&new_vec)
            .iter()
            .map(|member| member.as_str().unwrap())
            .collect();

        for member in hsm_member_vec {
            println!(
                "DEBUG - processing node {} in hsm group {}",
                member, hsm_group_name
            );
            if node_hsm_groups_hw_inventory_map.contains_key(member) {
                println!(
                    "DEBUG - node {} already processed for hsm groups {:?}",
                    member,
                    node_hsm_groups_hw_inventory_map.get(member).unwrap().0
                );

                node_hsm_groups_hw_inventory_map
                    .get_mut(member)
                    .unwrap()
                    .0
                    .push(&hsm_group_name);
            } else {
                println!(
                    "DEBUG - fetching hw components for node {} in hsm group {}",
                    member, hsm_group_name
                );
                let hw_inventory = get_node_hw_component_count(
                    shasta_token.to_string(),
                    shasta_base_url.to_string(),
                    shasta_root_cert.clone(),
                    member,
                    Vec::new(),
                )
                .await;

                node_hsm_groups_hw_inventory_map.insert(
                    member,
                    (vec![hsm_group_name], hw_inventory.1, hw_inventory.2),
                );
            }
        }
    }

    println!("\n************************************\nDEBUG - HW COMPONENT SUMMARY:\n",);

    let mut hsm_memory_capacity_lcm = u64::MAX;

    for (node, hsm_groups_hw_inventory) in node_hsm_groups_hw_inventory_map {
        let node_memory_capacity_lcm = utils::calculate_lcm(&hsm_groups_hw_inventory.2);
        if node_memory_capacity_lcm < hsm_memory_capacity_lcm {
            hsm_memory_capacity_lcm = node_memory_capacity_lcm;
        }
        println!(
            "DEBUG - node {} hwm groups {:?} hw inventory {:?} memory dimms capacity {:?} lcm {}",
            node,
            hsm_groups_hw_inventory.0,
            hsm_groups_hw_inventory.1,
            hsm_groups_hw_inventory.2,
            node_memory_capacity_lcm
        );
    }

    println!("Query LCM: {}", hsm_memory_capacity_lcm);
}

pub fn test_hsm_hw_management() {
    let hsm_zinal_hw_counters = vec![
        (
            "x1001c1s5b0n0",
            HashMap::from([("Memory 16384", 16), ("epyc", 2)]),
        ),
        (
            "x1001c1s5b0n1",
            HashMap::from([("Memory 16384", 16), ("epyc", 2)]),
        ),
        (
            "x1001c1s5b1n0",
            HashMap::from([("Memory 16384", 16), ("epyc", 2)]),
        ),
        (
            "x1001c1s5b1n1",
            HashMap::from([("epyc", 2), ("Memory 16384", 16)]),
        ),
        (
            "x1001c1s6b0n0",
            HashMap::from([("epyc", 2), ("Memory 16384", 15)]),
        ),
        (
            "x1001c1s6b0n1",
            HashMap::from([("epyc", 2), ("Memory 16384", 16)]),
        ),
        (
            "x1001c1s6b1n0",
            HashMap::from([("Memory 16384", 16), ("epyc", 2)]),
        ),
        (
            "x1001c1s6b1n1",
            HashMap::from([("epyc", 2), ("Memory 16384", 16)]),
        ),
        (
            "x1001c1s7b0n0",
            HashMap::from([("Memory 16384", 16), ("epyc", 2)]),
        ),
        (
            "x1001c1s7b0n1",
            HashMap::from([("Memory 16384", 16), ("epyc", 2)]),
        ),
        (
            "x1001c1s7b1n0",
            HashMap::from([("epyc", 2), ("Memory 16384", 16)]),
        ),
        (
            "x1001c1s7b1n1",
            HashMap::from([("Memory 16384", 16), ("epyc", 2)]),
        ),
        (
            "x1005c0s4b0n0",
            HashMap::from([("a100", 4), ("epyc", 1), ("Memory 16384", 32)]),
        ),
        (
            "x1005c0s4b0n1",
            HashMap::from([("epyc", 1), ("Memory 16384", 32), ("a100", 4)]),
        ),
        (
            "x1006c1s4b0n0",
            HashMap::from([("instinct", 8), ("Memory 16384", 32), ("epyc", 1)]),
        ),
        (
            "x1006c1s4b1n0",
            HashMap::from([("instinct", 8), ("epyc", 1), ("Memory 16384", 32)]),
        ),
    ];

    let hsm_nodes_free_hw_conters = vec![
        (
            "x1000c1s7b0n0",
            HashMap::from([("epyc", 2), ("Memory 16384", 16)]),
        ),
        (
            "x1000c1s7b0n1",
            HashMap::from([("Memory 16384", 16), ("epyc", 2)]),
        ),
        (
            "x1000c1s7b1n0",
            HashMap::from([("epyc", 2), ("Memory 16384", 16)]),
        ),
        (
            "x1000c1s7b1n1",
            HashMap::from([("epyc", 2), ("Memory 16384", 16)]),
        ),
        (
            "x1001c1s1b0n0",
            HashMap::from([("Memory 16384", 16), ("epyc", 2)]),
        ),
        (
            "x1001c1s1b0n1",
            HashMap::from([("Memory 16384", 16), ("epyc", 2)]),
        ),
        (
            "x1001c1s1b1n0",
            HashMap::from([("epyc", 2), ("Memory 16384", 16)]),
        ),
        (
            "x1001c1s1b1n1",
            HashMap::from([("epyc", 2), ("Memory 16384", 16)]),
        ),
        (
            "x1001c1s2b0n0",
            HashMap::from([("epyc", 2), ("Memory 16384", 16)]),
        ),
        (
            "x1001c1s2b0n1",
            HashMap::from([("epyc", 2), ("Memory 16384", 16)]),
        ),
        (
            "x1001c1s4b0n0",
            HashMap::from([("Memory 16384", 16), ("epyc", 2)]),
        ),
        (
            "x1001c1s4b0n1",
            HashMap::from([("epyc", 2), ("Memory 16384", 16)]),
        ),
        (
            "x1001c1s4b1n0",
            HashMap::from([("epyc", 2), ("Memory 16384", 16)]),
        ),
        (
            "x1001c1s4b1n1",
            HashMap::from([("Memory 16384", 16), ("epyc", 2)]),
        ),
    ];
}
