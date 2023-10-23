use serde_json::json;
use std::{collections::HashMap, sync::Arc, time::Instant};
use tokio::sync::Semaphore;

use crate::{
    cli::commands::apply_hsm_based_on_component_quantity::utils::{
        calculate_all_deltas, calculate_scores_and_normalized_scores, get_hsm_hw_component_count,
        hsm_node_hw_profile, node_migration,
    },
    shasta::hsm,
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

pub async fn exec(
    shasta_token: &str,
    shasta_base_url: &str,
    shasta_root_cert: &[u8],
    pattern: &str,
    parent_hsm_group_name: &str,
) {
    let mut target_hsm_group_hw_component_vec = Vec::new();

    // Normalize text in lowercase and separate each HSM group hw inventory pattern
    let pattern_lowercase = pattern.to_lowercase();

    let mut pattern_element_vec: Vec<&str> = pattern_lowercase.split(':').collect();

    let target_hsm_group_name = pattern_element_vec.remove(0);

    let user_defined_hw_component_counter_hashmap: &mut HashMap<String, usize> =
        &mut HashMap::new();

    // Check user input is correct
    for hw_component_counter in pattern_element_vec.chunks(2) {
        if hw_component_counter[0].parse::<String>().is_ok()
            && hw_component_counter[1].parse::<u8>().is_ok()
        {
            user_defined_hw_component_counter_hashmap.insert(
                hw_component_counter[0].parse::<String>().unwrap(),
                hw_component_counter[1].parse::<usize>().unwrap(),
            );
        } else {
            log::error!("Error in pattern. Please make sure to follow <hw component>:<counter>:... eg a100:4:epyc:10:instinct:8");
        }
    }

    println!(
        "User defined hw components with counters: {:?}",
        user_defined_hw_component_counter_hashmap
    );

    let mut user_defined_hw_component_vec: Vec<String> = user_defined_hw_component_counter_hashmap
        .keys()
        .cloned()
        .collect();

    user_defined_hw_component_vec.sort();

    let user_defined_hw_component_vec = user_defined_hw_component_vec;

    // *********************************************************************************************************
    // TARGET HSM GROUP

    // Get target HSM group details
    let hsm_group_target_value_rslt = hsm::http_client::get_hsm_group(
        shasta_token,
        shasta_base_url,
        shasta_root_cert,
        target_hsm_group_name,
    )
    .await;

    let hsm_group_target_value = match hsm_group_target_value_rslt {
        Err(_) => json!({
            "label": target_hsm_group_name,
            "description": "",
            "members": {
                "ids": []
            }
        }),
        Ok(hsm_group_target_value) => hsm_group_target_value,
    };

    // Get target HSM group members
    let hsm_group_target_members =
        hsm::utils::get_members_from_hsm_group_serde_value(&hsm_group_target_value);

    // Get HSM group members hw configurfation based on user input
    let start = Instant::now();

    let mut tasks = tokio::task::JoinSet::new();

    let sem = Arc::new(Semaphore::new(5)); // CSM 1.3.1 higher number of concurrent tasks won't
                                           // make it faster

    // Get HW inventory details for target HSM group
    for hsm_member in hsm_group_target_members.clone() {
        let shasta_token_string = shasta_token.to_string(); // TODO: make it static
        let shasta_base_url_string = shasta_base_url.to_string(); // TODO: make it static
        let shasta_root_cert_vec = shasta_root_cert.to_vec();
        let user_defined_hw_component_vec = user_defined_hw_component_counter_hashmap
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .clone();

        let permit = Arc::clone(&sem).acquire_owned().await;

        // println!("user_defined_hw_profile_vec_aux: {:?}", user_defined_hw_profile_vec_aux);
        tasks.spawn(async move {
            let _permit = permit; // Wait semaphore to allow new tasks https://github.com/tokio-rs/tokio/discussions/2648#discussioncomment-34885
            hsm_node_hw_profile(
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
        if let Ok(mut node_hw_property_vec_tuple) = message {
            node_hw_property_vec_tuple.1.sort();
            let mut counts: HashMap<String, usize> = HashMap::new();

            for node_hw_property_vec in node_hw_property_vec_tuple.1 {
                let count = counts.entry(node_hw_property_vec).or_insert(0);
                *count += 1;
            }

            target_hsm_group_hw_component_vec.push((node_hw_property_vec_tuple.0, counts));
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
    target_hsm_group_hw_component_vec
        .sort_by_key(|target_hsm_group_hw_component| target_hsm_group_hw_component.0.clone());

    // Calculate HSM group hw component counters
    let target_hsm_hw_component_count_hashmap = get_hsm_hw_component_count(
        &user_defined_hw_component_vec,
        &target_hsm_group_hw_component_vec,
    );

    println!(
        "HSM '{}' hw component counters: {:?}",
        target_hsm_group_name, target_hsm_hw_component_count_hashmap
    );

    // Calculate density scores
    let mut target_hsm_density_score_hashmap: HashMap<String, usize> = HashMap::new();
    for target_hsm_group_hw_component in &target_hsm_group_hw_component_vec {
        let counter = target_hsm_group_hw_component.1.values().sum();
        target_hsm_density_score_hashmap.insert(target_hsm_group_hw_component.clone().0, counter);
    }

    // *********************************************************************************************************
    // PARENT HSM GROUP
    // Get parent HSM group details
    let mut parent_hsm_group_hw_component_vec = Vec::new();
    let hsm_group_parent_value = hsm::http_client::get_hsm_group(
        shasta_token,
        shasta_base_url,
        shasta_root_cert,
        parent_hsm_group_name,
    )
    .await
    .unwrap();

    // Get target HSM group members
    let hsm_group_parent_members =
        hsm::utils::get_members_from_hsm_group_serde_value(&hsm_group_parent_value);

    // Get HSM group members hw configurfation based on user input
    let start = Instant::now();

    let mut tasks = tokio::task::JoinSet::new();

    // Get HW inventory details for parent HSM group
    for hsm_member in hsm_group_parent_members.clone() {
        let shasta_token_string = shasta_token.to_string();
        let shasta_base_url_string = shasta_base_url.to_string();
        let shasta_root_cert_vec = shasta_root_cert.to_vec();
        let user_defined_hw_component_vec = user_defined_hw_component_counter_hashmap
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .clone();
        // println!("user_defined_hw_profile_vec_aux: {:?}", user_defined_hw_profile_vec_aux);
        tasks.spawn(async move {
            hsm_node_hw_profile(
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
        if let Ok(mut node_hw_property_vec_tuple) = message {
            node_hw_property_vec_tuple.1.sort();
            let mut counts: HashMap<String, usize> = HashMap::new();

            for node_hw_property_vec in node_hw_property_vec_tuple.1 {
                let count = counts.entry(node_hw_property_vec).or_insert(0);
                *count += 1;
            }

            parent_hsm_group_hw_component_vec.push((node_hw_property_vec_tuple.0, counts));
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
    parent_hsm_group_hw_component_vec
        .sort_by_key(|target_hsm_group_hw_component| target_hsm_group_hw_component.0.clone());

    // Calculate HSM group hw component counters
    let parent_hsm_hw_component_count_hashmap = get_hsm_hw_component_count(
        &user_defined_hw_component_vec,
        &parent_hsm_group_hw_component_vec,
    );

    println!(
        "HSM '{}' hw component counters: {:?}",
        parent_hsm_group_name, parent_hsm_hw_component_count_hashmap
    );

    // Calculate density scores
    let mut parent_hsm_density_score_hashmap: HashMap<String, usize> = HashMap::new();
    for parent_hsm_group_hw_component in &parent_hsm_group_hw_component_vec {
        let counter = parent_hsm_group_hw_component.1.values().sum();
        parent_hsm_density_score_hashmap.insert(parent_hsm_group_hw_component.clone().0, counter);
    }

    // *********************************************************************************************************
    // HSM UPDATES

    let (
        hw_components_to_migrate_from_target_hsm_to_parent_hsm,
        hw_components_to_migrate_from_parent_hsm_to_target_hsm,
    ) = calculate_all_deltas(
        user_defined_hw_component_counter_hashmap,
        &target_hsm_hw_component_count_hashmap,
    );

    println!(
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
    );

    // Check parent HSM has enough capacity to handle user request
    for (hw_component, quantity) in &hw_components_to_migrate_from_parent_hsm_to_target_hsm {
        let resources_letf_in_parent_hsm_group = (*parent_hsm_hw_component_count_hashmap
            .get(hw_component)
            .unwrap() as isize)
            + quantity;
        if resources_letf_in_parent_hsm_group < 0 {
            eprintln!("Parent HSM {} does not have enough resources to fulfill user request. Not enough {} ({}). Exit", parent_hsm_group_name, hw_component, resources_letf_in_parent_hsm_group);
            std::process::exit(1);
        }
    }

    println!(
        "\n----- TARGET HSM GROUP '{}' -----\n",
        target_hsm_group_name
    );

    // Calculate initial scores
    let target_hsm_score_vec = calculate_scores_and_normalized_scores(
        &target_hsm_group_hw_component_vec,
        &hw_components_to_migrate_from_target_hsm_to_parent_hsm,
    );

    // Migrate nodes
    let nodes_to_remove_in_target_hsm = node_migration(
        user_defined_hw_component_counter_hashmap,
        &user_defined_hw_component_vec,
        &mut target_hsm_group_hw_component_vec,
        &target_hsm_density_score_hashmap,
        target_hsm_score_vec,
        hw_components_to_migrate_from_target_hsm_to_parent_hsm,
    );

    println!("----- PARENT HSM GROUP '{}' -----", parent_hsm_group_name);

    println!(
        "Components to move from '{}' to '{}' --> {:?}",
        parent_hsm_group_name,
        target_hsm_group_name,
        hw_components_to_migrate_from_parent_hsm_to_target_hsm
    );

    // Calculate initial scores
    let parent_hsm_score_vec = calculate_scores_and_normalized_scores(
        &parent_hsm_group_hw_component_vec,
        &hw_components_to_migrate_from_parent_hsm_to_target_hsm,
    );

    // Migrate nodes
    let nodes_to_remove_in_parent_hsm = node_migration(
        user_defined_hw_component_counter_hashmap,
        &user_defined_hw_component_vec,
        &mut parent_hsm_group_hw_component_vec,
        &parent_hsm_density_score_hashmap,
        parent_hsm_score_vec,
        hw_components_to_migrate_from_parent_hsm_to_target_hsm,
    );

    let mut new_target_hsm_members = hsm_group_target_members
        .iter()
        .filter(|node| !nodes_to_remove_in_target_hsm.contains(node))
        .cloned()
        .collect::<Vec<String>>();

    new_target_hsm_members = [
        new_target_hsm_members,
        nodes_to_remove_in_parent_hsm.clone(),
    ]
    .concat();

    new_target_hsm_members.sort();

    println!(
        "HSM group '{}' members: {}",
        target_hsm_group_name,
        new_target_hsm_members.join(",")
    );

    let mut new_parent_hsm_members = hsm_group_parent_members
        .iter()
        .filter(|node| !nodes_to_remove_in_parent_hsm.contains(node))
        .cloned()
        .collect::<Vec<String>>();

    new_parent_hsm_members = [new_parent_hsm_members, nodes_to_remove_in_target_hsm].concat();

    new_parent_hsm_members.sort();

    println!(
        "HSM group '{}' members: {}",
        parent_hsm_group_name,
        new_parent_hsm_members.join(",")
    );
}

pub mod utils {
    use std::collections::HashMap;

    use comfy_table::Color;
    use serde_json::Value;

    use crate::shasta::hsm;

    /// Removes as much nodes as it can from the target HSM group
    /// Returns a tuple with 2 vecs, the left one is the new target HSM group while the left one is
    /// the one containing the nodes removed from the target HSM
    pub fn node_migration(
        user_defined_hw_component_counter_vec: &HashMap<String, usize>,
        user_defined_hw_component_vec: &Vec<String>,
        target_hsm_group_hw_component_vec: &mut Vec<(String, HashMap<String, usize>)>,
        target_hsm_density_score_hashmap: &HashMap<String, usize>,
        mut target_hsm_score_vec: Vec<(String, isize)>,
        mut hw_components_to_migrate_from_target_hsm_to_parent_hsm: HashMap<String, isize>,
    ) -> Vec<String> {
        if target_hsm_score_vec.is_empty() {
            log::info!("No candidates to choose from");
            return Vec::new();
        }

        ////////////////////////////////
        // Initialize

        let mut nodes_migrated_from_target_hsm: Vec<String> = Vec::new();

        // Get best candidate
        let (mut best_candidate, mut best_candidate_counters) = get_best_candidate_to_migrate(
            &mut target_hsm_score_vec,
            target_hsm_group_hw_component_vec,
        );

        // Check if we need to keep iterating
        let mut work_to_do = keep_iterating(
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
                user_defined_hw_component_counter_vec
            );
            // Calculate HSM group hw component counters
            let target_hsm_hw_component_count_hashmap = get_hsm_hw_component_count(
                user_defined_hw_component_vec,
                target_hsm_group_hw_component_vec,
            );
            println!(
                "HSM group hw component counters: {:?}",
                target_hsm_hw_component_count_hashmap
            );
            println!(
                "HW component counters yet to remove: {:?}",
                hw_components_to_migrate_from_target_hsm_to_parent_hsm
            );
            println!(
                "Best candidate is '{}' with score {} and hw component counters {:?}\n",
                best_candidate.0,
                target_hsm_score_vec
                    .iter()
                    .find(|(node, _score)| node.eq(&best_candidate.0))
                    .unwrap()
                    .1,
                best_candidate_counters
            );

            // Print target hsm group hw configuration in table
            print_table(
                user_defined_hw_component_vec,
                target_hsm_group_hw_component_vec,
                target_hsm_density_score_hashmap,
                &target_hsm_score_vec,
            );

            ////////////////////////////////
            // Apply changes - Migrate from target to parent HSM

            // Add best candidate to parent HSM group
            nodes_migrated_from_target_hsm.push(best_candidate.0.clone());

            // Remove best candidate from target HSM group
            target_hsm_group_hw_component_vec.retain(|(node, _)| !node.eq(&best_candidate.0));

            // Update hw component counters to migrate
            hw_components_to_migrate_from_target_hsm_to_parent_hsm =
                update_user_defined_hw_component_counters(
                    &hw_components_to_migrate_from_target_hsm_to_parent_hsm,
                    &best_candidate_counters,
                );

            // Update scores
            target_hsm_score_vec = calculate_scores_and_normalized_scores(
                target_hsm_group_hw_component_vec,
                &hw_components_to_migrate_from_target_hsm_to_parent_hsm,
            );

            // Get best candidate
            (best_candidate, best_candidate_counters) = get_best_candidate_to_migrate(
                &mut target_hsm_score_vec,
                target_hsm_group_hw_component_vec,
            );

            // Check if we need to keep iterating
            work_to_do = keep_iterating(
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
        print_table(
            user_defined_hw_component_vec,
            target_hsm_group_hw_component_vec,
            target_hsm_density_score_hashmap,
            &target_hsm_score_vec,
        );

        nodes_migrated_from_target_hsm
    }

    pub fn keep_iterating(
        hw_components_to_migrate_from_target_hsm_to_parent_hsm: &HashMap<String, isize>,
        best_candidate_counters: &HashMap<String, usize>,
    ) -> bool {
        let mut work_to_do = false;
        for (hw_component, quantity) in hw_components_to_migrate_from_target_hsm_to_parent_hsm {
            if quantity.abs() > 0
                && best_candidate_counters.get(hw_component).is_some()
                && best_candidate_counters.get(hw_component).unwrap() <= &(quantity.unsigned_abs())
            {
                work_to_do = true;
                break;
            }
        }

        work_to_do
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

    pub fn calculate_scores_and_normalized_scores(
        target_hsm_group_hw_component_vec: &Vec<(String, HashMap<String, usize>)>,
        hw_components_to_migrate_from_target_hsm_to_parent_hsm: &HashMap<String, isize>,
    ) -> Vec<(String, isize)> {
        // Calculate HSM scores
        let target_hsm_score_vec: Vec<(String, isize)> = calculate_scores(
            target_hsm_group_hw_component_vec,
            hw_components_to_migrate_from_target_hsm_to_parent_hsm,
        );

        target_hsm_score_vec
    }

    pub fn get_best_candidate_to_migrate(
        hsm_normalize_score_vec: &mut [(String, isize)],
        hsm_group_hw_component_vec: &[(String, HashMap<String, usize>)],
    ) -> ((String, isize), HashMap<String, usize>) {
        hsm_normalize_score_vec.sort_by(|b, a| a.1.partial_cmp(&b.1).unwrap());

        // Get node with highest normalized score (best candidate)
        let highest_normalized_score: isize = *hsm_normalize_score_vec
            .iter()
            .map(|(_, normalized_score)| normalized_score)
            .max()
            .unwrap();

        let best_candidate: (String, isize) = hsm_normalize_score_vec
            .iter()
            .find(|(_, normalized_score)| *normalized_score == highest_normalized_score)
            .unwrap()
            .clone();

        let best_candidate_counters = &hsm_group_hw_component_vec
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

    pub fn calculate_scores(
        node_hw_component_counter_vec: &Vec<(String, HashMap<String, usize>)>,
        hw_components_to_migrate_from_one_hsm_to_another_hsm: &HashMap<String, isize>,
    ) -> Vec<(String, isize)> {
        let mut target_hsm_score_vec: Vec<(String, isize)> = Vec::new();

        for (node, hw_components) in node_hw_component_counter_vec {
            // println!("# Processing node: ({},{:?})", node, hw_components);

            // Calculate node's score
            let mut score = 0;
            for (hw_component, quantity) in hw_components {
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
                    total_number_of_hw_components_to_remove
                } else {
                    &0
                };
                // .get(hw_component)
                // .unwrap_or(&0)
                // .clone();

                /* println!(
                    "component {}, delta {}, component score {}",
                    hw_component, component_delta, quantity
                ); */
                let component_score = if component_delta.unsigned_abs() >= *quantity {
                    *quantity as isize
                    /* println!(
                        "hw component {} --> current score {} + node component score {} ==> new score {}",
                        hw_component, score, node_component_score, (score + component_score)
                    ); */
                } else {
                    // score for this component is 0
                    /* println!(
                        "delta ({}) < node component quantity ({}) --> node component score = 0",
                        component_delta, quantity
                    ); */
                    -(*quantity as isize) // This hw component type is in user
                                          // pattern but selecting this node as a candidate means the user would receive
                                          // less number of components that it initially requested
                };

                score += component_score;
            }

            target_hsm_score_vec.push((node.to_string(), score));
        }

        target_hsm_score_vec
    }

    pub fn calculate_all_deltas(
        user_defined_hw_component_counter_hashmap: &HashMap<String, usize>,
        hsm_hw_component_count_hashmap: &HashMap<String, usize>,
    ) -> (HashMap<String, isize>, HashMap<String, isize>) {
        let mut hw_components_to_migrate_from_target_hsm_to_parent_hsm: HashMap<String, isize> =
            HashMap::new();
        let mut hw_components_to_migrate_from_parent_hsm_to_target_hsm: HashMap<String, isize> =
            HashMap::new();
        for (user_defined_hw_component, new_quantity) in user_defined_hw_component_counter_hashmap {
            let quantity_from = hsm_hw_component_count_hashmap
                .get(user_defined_hw_component)
                .unwrap();

            let delta = (*new_quantity as isize) - (*quantity_from as isize);

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

    pub async fn hsm_node_hw_profile(
        shasta_token: String,
        shasta_base_url: String,
        shasta_root_cert: Vec<u8>,
        hsm_member: &str,
        user_defined_hw_profile_vec: Vec<String>,
    ) -> (String, Vec<String>) {
        let profile = hsm::http_client::get_hw_inventory(
            &shasta_token,
            &shasta_base_url,
            &shasta_root_cert,
            hsm_member,
        )
        .await
        .unwrap();
        let actual_xname_hw_profile_set =
            get_node_hw_properties(&profile, user_defined_hw_profile_vec.clone());

        (hsm_member.to_string(), actual_xname_hw_profile_set)
    }

    pub fn get_hsm_hw_component_count(
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

    /// Returns the properties in hw_property_list found in the node_hw_inventory_value
    pub fn get_node_hw_properties(
        node_hw_inventory_value: &Value,
        hw_component_pattern_list: Vec<String>,
    ) -> Vec<String> {
        let processor_vec =
            hsm::utils::get_list_processor_model_from_hw_inventory_value(node_hw_inventory_value)
                .unwrap_or_default();

        let accelerator_vec =
            hsm::utils::get_list_accelerator_model_from_hw_inventory_value(node_hw_inventory_value)
                .unwrap_or_default();

        let processor_and_accelerator = [processor_vec, accelerator_vec].concat();
        let processor_and_accelerator_lowercase = processor_and_accelerator
            .iter()
            .map(|hw_component| hw_component.to_lowercase());

        let mut node_hw_component_pattern_vec = Vec::new();
        for actual_hw_component_pattern in processor_and_accelerator_lowercase {
            for hw_component_pattern in &hw_component_pattern_list {
                if actual_hw_component_pattern.contains(hw_component_pattern) {
                    node_hw_component_pattern_vec.push(hw_component_pattern.to_string());
                }
            }
        }

        node_hw_component_pattern_vec
    }

    pub fn print_table(
        user_defined_hw_componet_vec: &Vec<String>,
        hsm_hw_pattern_vec: &[(String, HashMap<String, usize>)],
        hsm_density_score_hashmap: &HashMap<String, usize>,
        hsm_score_vec: &[(String, isize)],
    ) {
        let mut table = comfy_table::Table::new();

        table.set_header(
            [
                vec!["Node".to_string()],
                user_defined_hw_componet_vec.clone(),
                vec!["Density Score".to_string()],
                vec!["Score".to_string()],
            ]
            .concat(),
        );

        for hsm_hw_pattern in hsm_hw_pattern_vec {
            let xname = hsm_hw_pattern.0.to_string();
            let node_pattern_hashmap = hsm_hw_pattern.1.clone();
            let mut row: Vec<comfy_table::Cell> = Vec::new();
            // Node xname table cell
            row.push(
                comfy_table::Cell::new(xname.clone())
                    .set_alignment(comfy_table::CellAlignment::Center),
            );
            // User hw components table cell
            for user_defined_hw_component in user_defined_hw_componet_vec {
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
            }
            // Node density score table cell
            row.push(
                comfy_table::Cell::new(hsm_density_score_hashmap.get(&xname).unwrap())
                    .set_alignment(comfy_table::CellAlignment::Center),
            );
            // Node score table cell
            let node_score = hsm_score_vec
                .iter()
                .find(|(node_name, _)| node_name.eq(&xname))
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
}
