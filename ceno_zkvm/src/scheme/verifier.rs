use std::marker::PhantomData;
use std::fmt::{Display, Debug};

use ark_std::iterable::Iterable;
use ff_ext::ExtensionField;

use itertools::{izip, Itertools};
use mpcs::PolynomialCommitmentScheme;
use multilinear_extensions::{
    mle::{IntoMLE, MultilinearExtension},
    util::ceil_log2,
    virtual_poly::{build_eq_x_r_vec_sequential, eq_eval, VPAuxInfo},
};
use sumcheck::structs::{IOPProof, IOPVerifierState};
use transcript::Transcript;

use goldilocks::SmallField;

use crate::{
    error::ZKVMError,
    scheme::{
        constants::{NUM_FANIN, NUM_FANIN_LOGUP, SEL_DEGREE},
        utils::eval_by_expr_with_fixed,
    },
    structs::{Point, PointAndEval, TowerProofs, VerifyingKey, ZKVMVerifyingKey},
    utils::{eq_eval_less_or_equal_than, get_challenge_pows, next_pow2_instance_padding},
};

use super::{
    constants::MAINCONSTRAIN_SUMCHECK_BATCH_SIZE, utils::eval_by_expr, ZKVMOpcodeProof, ZKVMProof,
    ZKVMTableProof,
};

pub struct ZKVMVerifier<E: ExtensionField, PCS: PolynomialCommitmentScheme<E>> {
    pub(crate) vk: ZKVMVerifyingKey<E, PCS>,
}

fn print_list_as_input<I: Display>(name: &str, entries: &Vec<I>) {
    print!("{}: [", name);
    for e in entries {
        print!(" {}", e);
    }
    println!(" ]");
}

fn ext_field_as_limbs_no_trait<T: Debug>(scalar: &T) -> [String; 2] {
    let scalar_str = format!("{:?}", scalar);
    let str_seg: Vec<&str> = scalar_str.split(&['(', ')']).collect();
    [str_seg[2].to_string(), str_seg[4].to_string()]
}

impl<E: ExtensionField, PCS: PolynomialCommitmentScheme<E>> ZKVMVerifier<E, PCS> {
    pub fn new(vk: ZKVMVerifyingKey<E, PCS>) -> Self {
        ZKVMVerifier { vk }
    }

    pub fn verify_proof(
        &self,
        vm_proof: ZKVMProof<E, PCS>,
        mut transcript: Transcript<E>,
    ) -> Result<bool, ZKVMError> {
        // main invariant between opcode circuits and table circuits
        let mut prod_r = E::ONE;
        let mut prod_w = E::ONE;
        let mut logup_sum = E::ZERO;

        // write fixed commitment to transcript
        for (_, vk) in self.vk.circuit_vks.iter() {
            if let Some(fixed_commit) = vk.fixed_commit.as_ref() {
                PCS::write_commitment(fixed_commit, &mut transcript)
                    .map_err(ZKVMError::PCSError)?;
            }
        }

        for (_, (_, proof)) in vm_proof.opcode_proofs.iter() {
            PCS::write_commitment(&proof.wits_commit, &mut transcript)
                .map_err(ZKVMError::PCSError)?;
        }
        for (_, (_, proof)) in vm_proof.table_proofs.iter() {
            PCS::write_commitment(&proof.wits_commit, &mut transcript)
                .map_err(ZKVMError::PCSError)?;
        }

        // alpha, beta
        let challenges = [
            transcript.read_challenge().elements,
            transcript.read_challenge().elements,
        ];
        tracing::debug!("challenges: {:?}", challenges);

        let dummy_table_item = challenges[0];
        let mut dummy_table_item_multiplicity = 0;
        let point_eval = PointAndEval::default();
        let mut transcripts = transcript.fork(vm_proof.num_circuits());

        println!("NAMES: {:?}", self.vk.circuit_vks.keys());
        for (name, (i, opcode_proof)) in vm_proof.opcode_proofs {
            println!("NAME: {}", name);

            let transcript = &mut transcripts[i];

            let circuit_vk = self
                .vk
                .circuit_vks
                .get(&name)
                .ok_or(ZKVMError::VKNotFound(name.clone()))?;
            let _rand_point = self.verify_opcode_proof(
                &name,
                &self.vk.vp,
                circuit_vk,
                &opcode_proof,
                transcript,
                NUM_FANIN,
                &point_eval,
                &challenges,
            )?;
            tracing::info!("verified proof for opcode {}", name);

            // getting the number of dummy padding item that we used in this opcode circuit
            let num_lks = circuit_vk.get_cs().lk_expressions.len();
            let num_padded_lks_per_instance = next_pow2_instance_padding(num_lks) - num_lks;
            let num_padded_instance =
                next_pow2_instance_padding(opcode_proof.num_instances) - opcode_proof.num_instances;
            dummy_table_item_multiplicity += num_padded_lks_per_instance
                * opcode_proof.num_instances
                + num_lks.next_power_of_two() * num_padded_instance;

            prod_r *= opcode_proof.record_r_out_evals.iter().product::<E>();
            prod_w *= opcode_proof.record_w_out_evals.iter().product::<E>();

            logup_sum +=
                opcode_proof.lk_p1_out_eval * opcode_proof.lk_q1_out_eval.invert().unwrap();
            logup_sum +=
                opcode_proof.lk_p2_out_eval * opcode_proof.lk_q2_out_eval.invert().unwrap();
        }

        for (name, (i, table_proof)) in vm_proof.table_proofs {
            let transcript = &mut transcripts[i];

            let circuit_vk = self
                .vk
                .circuit_vks
                .get(&name)
                .ok_or(ZKVMError::VKNotFound(name.clone()))?;
            let _rand_point = self.verify_table_proof(
                &name,
                &self.vk.vp,
                circuit_vk,
                &table_proof,
                transcript,
                NUM_FANIN_LOGUP,
                &point_eval,
                &challenges,
            )?;
            tracing::info!("verified proof for table {}", name);

            logup_sum -= table_proof.lk_p1_out_eval * table_proof.lk_q1_out_eval.invert().unwrap();
            logup_sum -= table_proof.lk_p2_out_eval * table_proof.lk_q2_out_eval.invert().unwrap();
        }
        logup_sum -=
            E::from(dummy_table_item_multiplicity as u64) * dummy_table_item.invert().unwrap();

        // check rw_set equality across all proofs
        // TODO: enable this when we have cpu init/finalize and mem init/finalize
        // if prod_r != prod_w {
        //     return Err(ZKVMError::VerifyError("prod_r != prod_w".into()));
        // }

        // check logup relation across all proofs
        if logup_sum != E::ZERO {
            return Err(ZKVMError::VerifyError(format!(
                "logup_sum({:?}) != 0",
                logup_sum
            )));
        }

        Ok(true)
    }

    /// verify proof and return input opening point
    #[allow(clippy::too_many_arguments)]
    pub fn verify_opcode_proof(
        &self,
        name: &str,
        vp: &PCS::VerifierParam,
        circuit_vk: &VerifyingKey<E, PCS>,
        proof: &ZKVMOpcodeProof<E, PCS>,
        transcript: &mut Transcript<E>,
        num_product_fanin: usize,
        _out_evals: &PointAndEval<E>,
        challenges: &[E; 2], // derive challenge from PCS
    ) -> Result<Point<E>, ZKVMError> {
        // Number of mem cells required to express each struct in Zok
        const EXT_FIELD_WIDTH: usize = 2;
        const EXPRESSION_WIDTH: usize = 7 + 2 * EXT_FIELD_WIDTH;
        const CONSTRAINT_SYSTEM_WIDTH: usize = 22 + 2 * EXPRESSION_WIDTH;
        const VERIFYING_KEY_WIDTH: usize = CONSTRAINT_SYSTEM_WIDTH;

        println!("\n\n--\nINPUT:");
        // Divide memory into three regions
        // 1. All entries of all expressions: expr_concat_list
        // 2. All pointers to head of expressions: expr_pointer_list
        // 3. Pointers to head of r_expr, w_expr, etc. pointers: head_pointer_list
        let mut expr_concat_list = Vec::new();
        let mut expr_offset = 0;
        let mut expr_pointer_list = Vec::new();
        let mut head_offset = 0;
        let mut head_pointer_mat = Vec::new();
        let mut vk_count = 0;
        for (_, val) in &self.vk.circuit_vks {
            head_pointer_mat.push(Vec::new());
            // r_expr
            head_pointer_mat[vk_count].push(head_offset);
            for r_expr in &val.cs.r_expressions {
                let (r_len, r_list) = r_expr.expr_as_list();
                expr_pointer_list.push(expr_offset);
                expr_offset += r_len;
                expr_concat_list.extend(r_list);
            }
            head_offset += val.cs.r_expressions.len();
            // w_expr
            head_pointer_mat[vk_count].push(head_offset);
            for w_expr in &val.cs.w_expressions {
                let (w_len, w_list) = w_expr.expr_as_list();
                expr_pointer_list.push(expr_offset);
                expr_offset += w_len;
                expr_concat_list.extend(w_list);
            }
            head_offset += val.cs.w_expressions.len();
            // lk_expr
            head_pointer_mat[vk_count].push(head_offset);
            for lk_expr in &val.cs.lk_expressions {
                let (lk_len, lk_list) = lk_expr.expr_as_list();
                expr_pointer_list.push(expr_offset);
                expr_offset += lk_len;
                expr_concat_list.extend(lk_list);
            }
            head_offset += val.cs.lk_expressions.len();
            // lk_table_expr
            head_pointer_mat[vk_count].push(head_offset);
            for lk_table_expr in &val.cs.lk_table_expressions {
                let (mul_len, mul_list) = lk_table_expr.multiplicity.expr_as_list();
                expr_pointer_list.push(expr_offset);
                expr_offset += mul_len;
                expr_concat_list.extend(mul_list);
                let (val_len, val_list) = lk_table_expr.values.expr_as_list();
                expr_pointer_list.push(expr_offset);
                expr_offset += val_len;
                expr_concat_list.extend(val_list);
            }
            head_offset += 2 * val.cs.lk_table_expressions.len();
            // assert_zero_expr
            head_pointer_mat[vk_count].push(head_offset);
            for assert_zero_expr in &val.cs.assert_zero_expressions {
                let (assert_zero_len, assert_zero_list) = assert_zero_expr.expr_as_list();
                expr_pointer_list.push(expr_offset);
                expr_offset += assert_zero_len;
                expr_concat_list.extend(assert_zero_list);
            }
            head_offset += val.cs.assert_zero_expressions.len();
            // assert_zero_sumcheck_expr
            head_pointer_mat[vk_count].push(head_offset);
            for assert_zero_sumcheck_expr in &val.cs.assert_zero_sumcheck_expressions {
                let (assert_zero_sumcheck_len, assert_zero_sumcheck_list) = assert_zero_sumcheck_expr.expr_as_list();
                expr_pointer_list.push(expr_offset);
                expr_offset += assert_zero_sumcheck_len;
                expr_concat_list.extend(assert_zero_sumcheck_list);
            }
            head_offset += val.cs.assert_zero_sumcheck_expressions.len();
            // Record the last head_offset to obtain length from difference
            head_pointer_mat[vk_count].push(head_offset);
            // chip_record_alpha, single entry, push expr_offset directly to head_pointer
            head_pointer_mat[vk_count].push(expr_offset);
            let (cr_alpha_len, cr_alpha_list) = val.cs.chip_record_alpha.expr_as_list();
            expr_offset += cr_alpha_len;
            expr_concat_list.extend(cr_alpha_list);
            // chip_record_beta, single entry, push expr_offset directly to head_pointer
            head_pointer_mat[vk_count].push(expr_offset);
            let (cr_beta_len, cr_beta_list) = val.cs.chip_record_beta.expr_as_list();
            expr_offset += cr_beta_len;
            expr_concat_list.extend(cr_beta_list);

            vk_count += 1;
        }

        // Construct ConstraintSystem using the entries above
        let mut mem_offset = expr_concat_list.len() + expr_pointer_list.len();
        let mut cs_concat_list = Vec::new();
        for i in 0..self.vk.circuit_vks.len() {
            // r_expr, w_expr, lk_expr, az_expr
            for j in 0..head_pointer_mat[i].len() - 3 {
                // len
                cs_concat_list.push(head_pointer_mat[i][j + 1] - head_pointer_mat[i][j]);
                // pointer
                cs_concat_list.push(mem_offset + head_pointer_mat[i][j]);
            }
            // max_non_lc_degree
            cs_concat_list.push(self.vk.circuit_vks.iter().nth(i).unwrap().1.cs.max_non_lc_degree);
            // chip_record_alpha, chip_record_beta
            cs_concat_list.push(head_pointer_mat[i][head_pointer_mat[i].len() - 2]);
            cs_concat_list.push(head_pointer_mat[i][head_pointer_mat[i].len() - 1]);
        }

        // Print everything in self out
        print_list_as_input("expr_concat", &expr_concat_list);
        print_list_as_input("expr_pointer", &expr_pointer_list);
        let circuit_vks_len = self.vk.circuit_vks.len();
        println!("self^vk^circuit_vks_len: {}", circuit_vks_len);
        print_list_as_input("self^vk^circuit_vks_key", &cs_concat_list);
        mem_offset += cs_concat_list.len();

        // proof
        let mut proof_entries_concat = Vec::new();
        let mut next_proof_pointer = 0;
        let mut proof_pointers_mat = Vec::new();
        let mut head_pointers_list = vec![Vec::new(); 3];
        // proofs
        for t0 in &proof.tower_proof.proofs {
            head_pointers_list[0].push(next_proof_pointer);
            for t1 in t0 {
                proof_pointers_mat.push(mem_offset + proof_entries_concat.len());
                next_proof_pointer += 1;
                for e in &t1.evaluations {
                    proof_entries_concat.extend(ext_field_as_limbs_no_trait(&e));
                }
            }
        }
        // prod_specs_eval
        for t0 in &proof.tower_proof.prod_specs_eval {
            head_pointers_list[1].push(next_proof_pointer);
            for t1 in t0 {
                proof_pointers_mat.push(mem_offset + proof_entries_concat.len());
                next_proof_pointer += 1;
                for e in t1 {
                    proof_entries_concat.extend(ext_field_as_limbs_no_trait(&e));
                }
            }
        }
        // logup_specs_eval
        for t0 in &proof.tower_proof.logup_specs_eval {
            head_pointers_list[2].push(next_proof_pointer);
            for t1 in t0 {
                proof_pointers_mat.push(mem_offset + proof_entries_concat.len());
                next_proof_pointer += 1;
                for e in t1 {
                    proof_entries_concat.extend(ext_field_as_limbs_no_trait(&e));
                }
            }
        }
        mem_offset += proof_entries_concat.len();
        print_list_as_input("proof_entries_concat", &proof_entries_concat);
        print_list_as_input("proof_pointers_mat", &proof_pointers_mat);
        // main_sel_sumcheck_proofs
        let mut mssp_concat = Vec::new();
        let mut mssp_offset = 0;
        let mut mssp_pointers = Vec::new();
        for m in &proof.main_sel_sumcheck_proofs {
            mssp_pointers.push(mssp_offset);
            for e in &m.evaluations {
                mssp_concat.extend(ext_field_as_limbs_no_trait(e));
            }
            mssp_offset += m.evaluations.len();
        }
        print_list_as_input("main_sel_sumcheck_proofs_concat", &mssp_concat);
        mem_offset += mssp_concat.len();

        println!("proof^num_instances: {}", proof.num_instances);
        // record_r_out_evals, record_w_out_evals
        let mut record_r_out_evals = Vec::new();
        let mut record_w_out_evals = Vec::new();
        for r in &proof.record_r_out_evals {
            record_r_out_evals.extend(ext_field_as_limbs_no_trait(r));
        }
        for w in &proof.record_w_out_evals {
            record_w_out_evals.extend(ext_field_as_limbs_no_trait(w));
        }
        print_list_as_input("proof^record_r_out_evals", &record_r_out_evals);
        mem_offset += record_r_out_evals.len();
        print_list_as_input("proof^record_w_out_evals", &record_w_out_evals);
        mem_offset += record_w_out_evals.len();
        // lk_p1_out_eval, lk_p2_out_eval, lk_q1_out_eval, lk_q2_out_eval
        let lk_p1_out_eval = ext_field_as_limbs_no_trait(&proof.lk_p1_out_eval);
        let lk_p2_out_eval = ext_field_as_limbs_no_trait(&proof.lk_p2_out_eval);
        let lk_q1_out_eval = ext_field_as_limbs_no_trait(&proof.lk_q1_out_eval);
        let lk_q2_out_eval = ext_field_as_limbs_no_trait(&proof.lk_q2_out_eval);
        println!("proof^lk_p1_out_eval^b0: {}", lk_p1_out_eval[0]);
        println!("proof^lk_p1_out_eval^b1: {}", lk_p1_out_eval[1]);
        println!("proof^lk_p2_out_eval^b0: {}", lk_p2_out_eval[0]);
        println!("proof^lk_p2_out_eval^b1: {}", lk_p2_out_eval[1]);
        println!("proof^lk_q1_out_eval^b0: {}", lk_q1_out_eval[0]);
        println!("proof^lk_q1_out_eval^b1: {}", lk_q1_out_eval[1]);
        println!("proof^lk_q2_out_eval^b0: {}", lk_q2_out_eval[0]);
        println!("proof^lk_q2_out_eval^b1: {}", lk_q2_out_eval[1]);
        // tower_proof
        println!("proof^tower_proof^prod_spec_size: {}", proof.tower_proof.prod_spec_size());
        println!("proof^tower_proof^logup_spec_size: {}", proof.tower_proof.logup_spec_size());
        // proofs
        head_pointers_list[0] = head_pointers_list[0].iter().map(|i| mem_offset + i).collect();
        head_pointers_list[1] = head_pointers_list[1].iter().map(|i| mem_offset + i).collect();
        head_pointers_list[2] = head_pointers_list[2].iter().map(|i| mem_offset + i).collect();
        print_list_as_input("proof^tower_proof^proofs", &head_pointers_list[0]);
        print_list_as_input("proof^tower_proof^prod_specs_eval", &head_pointers_list[1]);
        print_list_as_input("proof^tower_proof^logup_specs_eval", &head_pointers_list[2]);
        mem_offset += head_pointers_list[0].len() + head_pointers_list[1].len() + head_pointers_list[2].len();

        // main_sel_sumcheck_proofs
        println!("proof^main_sel_sumcheck_proofs_len: {}", proof.main_sel_sumcheck_proofs.len());
        mssp_pointers = mssp_pointers.iter().map(|i| mem_offset + i).collect();
        print_list_as_input("proof^main_sel_sumcheck_proofs", &mssp_pointers);
        mem_offset += mssp_pointers.len();

        // r_records_in_evals, w_records_in_evals, lk_records_in_evals
        println!("proof^r_records_in_evals_len: {}", proof.r_records_in_evals.len());
        let mut r_records_in_evals = Vec::new();
        for r in &proof.r_records_in_evals {
            r_records_in_evals.extend(ext_field_as_limbs_no_trait(r));
        }
        print_list_as_input("proof^r_records_in_evals", &r_records_in_evals);
        mem_offset += r_records_in_evals.len();
        // w
        println!("proof^w_records_in_evals_len: {}", proof.w_records_in_evals.len());
        let mut w_records_in_evals = Vec::new();
        for w in &proof.w_records_in_evals {
            w_records_in_evals.extend(ext_field_as_limbs_no_trait(w));
        }
        print_list_as_input("proof^w_records_in_evals", &w_records_in_evals);
        mem_offset += w_records_in_evals.len();
        // lk
        println!("proof^lk_records_in_evals_len: {}", proof.lk_records_in_evals.len());
        let mut lk_records_in_evals = Vec::new();
        for lk in &proof.lk_records_in_evals {
            lk_records_in_evals.extend(ext_field_as_limbs_no_trait(lk));
        }
        print_list_as_input("proof^lk_records_in_evals", &lk_records_in_evals);
        mem_offset += lk_records_in_evals.len();

        // wits_in_evals
        println!("proof^wits_in_evals_len: {}", proof.wits_in_evals.len());
        let mut wits_in_evals = Vec::new();
        for w in &proof.wits_in_evals {
            wits_in_evals.extend(ext_field_as_limbs_no_trait(w));
        }
        print_list_as_input("proof^wits_in_evals", &wits_in_evals);
        mem_offset += wits_in_evals.len();

        // transcript
        print!("t^permutation^state: [ ");
        for f in &transcript.permutation.state {
            print!("{} ", f.to_canonical_u64());
        }
        println!("]");

        // num_product_fanin, challenges
        println!("num_product_fanin: {}", num_product_fanin);
        let mut c_concat = Vec::new();
        for c in challenges {
            c_concat.extend(ext_field_as_limbs_no_trait(c));
        }
        print_list_as_input("challenges", &c_concat);

        println!("--\nWITNESSES:");

        // --
        // END PRINT
        // --

        let cs = circuit_vk.get_cs();
        let (r_counts_per_instance, w_counts_per_instance, lk_counts_per_instance) = (
            cs.r_expressions.len(),
            cs.w_expressions.len(),
            cs.lk_expressions.len(),
        );
        let (log2_r_count, log2_w_count, log2_lk_count) = (
            ceil_log2(r_counts_per_instance),
            ceil_log2(w_counts_per_instance),
            ceil_log2(lk_counts_per_instance),
        );
        let (chip_record_alpha, _) = (challenges[0], challenges[1]);

        let num_instances = proof.num_instances;
        let next_pow2_instance = next_pow2_instance_padding(num_instances);
        let log2_num_instances = ceil_log2(next_pow2_instance);

        // verify and reduce product tower sumcheck
        let tower_proofs = &proof.tower_proof;

        let (rt_tower, record_evals, logup_p_evals, logup_q_evals) = TowerVerify::verify(
            vec![
                proof.record_r_out_evals.clone(),
                proof.record_w_out_evals.clone(),
            ],
            vec![vec![
                proof.lk_p1_out_eval,
                proof.lk_p2_out_eval,
                proof.lk_q1_out_eval,
                proof.lk_q2_out_eval,
            ]],
            tower_proofs,
            vec![
                log2_num_instances + log2_r_count,
                log2_num_instances + log2_w_count,
                log2_num_instances + log2_lk_count,
            ],
            num_product_fanin,
            transcript,
        )?;
        assert!(record_evals.len() == 2, "[r_record, w_record]");
        assert!(logup_q_evals.len() == 1, "[lk_q_record]");
        assert!(logup_p_evals.len() == 1, "[lk_p_record]");

        // verify LogUp witness nominator p(x) ?= constant vector 1
        // index 0 is LogUp witness for Fixed Lookup table
        if logup_p_evals[0].eval != E::ONE {
            return Err(ZKVMError::VerifyError(
                "Lookup table witness p(x) != constant 1".into(),
            ));
        }

        // verify zero statement (degree > 1) + sel sumcheck
        let (rt_r, rt_w, rt_lk): (Vec<E>, Vec<E>, Vec<E>) = (
            record_evals[0].point.clone(),
            record_evals[1].point.clone(),
            logup_q_evals[0].point.clone(),
        );

        let alpha_pow = get_challenge_pows(
            MAINCONSTRAIN_SUMCHECK_BATCH_SIZE + cs.assert_zero_sumcheck_expressions.len(),
            transcript,
        );
        let mut alpha_pow_iter = alpha_pow.iter();
        let (alpha_read, alpha_write, alpha_lk) = (
            alpha_pow_iter.next().unwrap(),
            alpha_pow_iter.next().unwrap(),
            alpha_pow_iter.next().unwrap(),
        );
        // alpha_read * (out_r[rt] - 1) + alpha_write * (out_w[rt] - 1) + alpha_lk * (out_lk_q - chip_record_alpha)
        // + 0 // 0 come from zero check
        let claim_sum = *alpha_read * (record_evals[0].eval - E::ONE)
            + *alpha_write * (record_evals[1].eval - E::ONE)
            + *alpha_lk * (logup_q_evals[0].eval - chip_record_alpha);

        let main_sel_subclaim = IOPVerifierState::verify(
            claim_sum,
            &IOPProof {
                point: vec![], // final claimed point will be derive from sumcheck protocol
                proofs: proof.main_sel_sumcheck_proofs.clone(),
            },
            &VPAuxInfo {
                // + 1 from sel_non_lc_zero_sumcheck
                max_degree: SEL_DEGREE.max(cs.max_non_lc_degree + 1),
                num_variables: log2_num_instances,
                phantom: PhantomData,
            },
            transcript,
        );
        let (input_opening_point, expected_evaluation) = (
            main_sel_subclaim
                .point
                .iter()
                .map(|c| c.elements)
                .collect_vec(),
            main_sel_subclaim.expected_evaluation,
        );
        let eq_r = build_eq_x_r_vec_sequential(&rt_r[..log2_r_count]);
        let eq_w = build_eq_x_r_vec_sequential(&rt_w[..log2_w_count]);
        let eq_lk = build_eq_x_r_vec_sequential(&rt_lk[..log2_lk_count]);

        let (sel_r, sel_w, sel_lk, sel_non_lc_zero_sumcheck) = {
            // sel(rt, t)
            (
                eq_eval_less_or_equal_than(
                    num_instances - 1,
                    &input_opening_point,
                    &rt_r[log2_r_count..],
                ),
                eq_eval_less_or_equal_than(
                    num_instances - 1,
                    &input_opening_point,
                    &rt_w[log2_w_count..],
                ),
                eq_eval_less_or_equal_than(
                    num_instances - 1,
                    &input_opening_point,
                    &rt_lk[log2_lk_count..],
                ),
                // only initialize when circuit got non empty assert_zero_sumcheck_expressions
                {
                    let rt_non_lc_sumcheck = rt_tower[..log2_num_instances].to_vec();
                    if !cs.assert_zero_sumcheck_expressions.is_empty() {
                        Some(eq_eval_less_or_equal_than(
                            num_instances - 1,
                            &input_opening_point,
                            &rt_non_lc_sumcheck,
                        ))
                    } else {
                        None
                    }
                },
            )
        };

        let computed_evals = [
            // read
            *alpha_read
                * sel_r
                * ((0..r_counts_per_instance)
                    .map(|i| proof.r_records_in_evals[i] * eq_r[i])
                    .sum::<E>()
                    + eq_r[r_counts_per_instance..].iter().sum::<E>()
                    - E::ONE),
            // write
            *alpha_write
                * sel_w
                * ((0..w_counts_per_instance)
                    .map(|i| proof.w_records_in_evals[i] * eq_w[i])
                    .sum::<E>()
                    + eq_w[w_counts_per_instance..].iter().sum::<E>()
                    - E::ONE),
            // lookup
            *alpha_lk
                * sel_lk
                * ((0..lk_counts_per_instance)
                    .map(|i| proof.lk_records_in_evals[i] * eq_lk[i])
                    .sum::<E>()
                    + chip_record_alpha
                        * (eq_lk[lk_counts_per_instance..].iter().sum::<E>() - E::ONE)),
            // degree > 1 zero exp sumcheck
            {
                // sel(rt_non_lc_sumcheck, main_sel_eval_point) * \sum_j (alpha{j} * expr(main_sel_eval_point))
                sel_non_lc_zero_sumcheck.unwrap_or(E::ZERO)
                    * cs.assert_zero_sumcheck_expressions
                        .iter()
                        .zip_eq(alpha_pow_iter)
                        .map(|(expr, alpha)| {
                            // evaluate zero expression by all wits_in_evals because they share the unique input_opening_point opening
                            *alpha * eval_by_expr(&proof.wits_in_evals, challenges, expr)
                        })
                        .sum::<E>()
            },
        ]
        .iter()
        .sum::<E>();
        if computed_evals != expected_evaluation {
            return Err(ZKVMError::VerifyError(
                "main + sel evaluation verify failed".into(),
            ));
        }
        // verify records (degree = 1) statement, thus no sumcheck
        if cs
            .r_expressions
            .iter()
            .chain(cs.w_expressions.iter())
            .chain(cs.lk_expressions.iter())
            .zip_eq(
                proof.r_records_in_evals[..r_counts_per_instance]
                    .iter()
                    .chain(proof.w_records_in_evals[..w_counts_per_instance].iter())
                    .chain(proof.lk_records_in_evals[..lk_counts_per_instance].iter()),
            )
            .any(|(expr, expected_evals)| {
                eval_by_expr(&proof.wits_in_evals, challenges, expr) != *expected_evals
            })
        {
            return Err(ZKVMError::VerifyError(
                "record evaluate != expected_evals".into(),
            ));
        }

        // verify zero expression (degree = 1) statement, thus no sumcheck
        if cs
            .assert_zero_expressions
            .iter()
            .any(|expr| eval_by_expr(&proof.wits_in_evals, challenges, expr) != E::ZERO)
        {
            // TODO add me back
            // return Err(ZKVMError::VerifyError("zero expression != 0"));
        }

        tracing::debug!(
            "[opcode {}] verify opening proof for {} polys at {:?}",
            name,
            proof.wits_in_evals.len(),
            input_opening_point
        );
        PCS::simple_batch_verify(
            vp,
            &proof.wits_commit,
            &input_opening_point,
            &proof.wits_in_evals,
            &proof.wits_opening_proof,
            transcript,
        )
        .map_err(ZKVMError::PCSError)?;

        Ok(input_opening_point)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn verify_table_proof(
        &self,
        name: &str,
        vp: &PCS::VerifierParam,
        circuit_vk: &VerifyingKey<E, PCS>,
        proof: &ZKVMTableProof<E, PCS>,
        transcript: &mut Transcript<E>,
        num_logup_fanin: usize,
        _out_evals: &PointAndEval<E>,
        challenges: &[E; 2],
    ) -> Result<Point<E>, ZKVMError> {
        let cs = circuit_vk.get_cs();
        let lk_counts_per_instance = cs.lk_table_expressions.len();
        let log2_lk_count = ceil_log2(lk_counts_per_instance);

        let num_instances = proof.num_instances;
        let log2_num_instances = ceil_log2(num_instances);

        // verify and reduce product tower sumcheck
        let tower_proofs = &proof.tower_proof;

        let expected_max_round = log2_num_instances + log2_lk_count;
        let (_, _, logup_p_evals, logup_q_evals) = TowerVerify::verify(
            vec![],
            vec![vec![
                proof.lk_p1_out_eval,
                proof.lk_p2_out_eval,
                proof.lk_q1_out_eval,
                proof.lk_q2_out_eval,
            ]],
            tower_proofs,
            vec![expected_max_round],
            num_logup_fanin,
            transcript,
        )?;
        assert!(logup_q_evals.len() == 1, "[lk_q_record]");
        assert!(logup_p_evals.len() == 1, "[lk_p_record]");
        assert_eq!(logup_p_evals[0].point, logup_q_evals[0].point);

        // verify selector layer sumcheck
        let rt_lk: Vec<E> = logup_p_evals[0].point.to_vec();

        // 2 for denominator and numerator
        let alpha_pow = get_challenge_pows(2, transcript);
        let mut alpha_pow_iter = alpha_pow.iter();
        let (alpha_lk_d, alpha_lk_n) = (
            alpha_pow_iter.next().unwrap(),
            alpha_pow_iter.next().unwrap(),
        );
        // alpha_lk * (out_lk_q - one) + alpha_lk_n * out_lk_p
        let claim_sum =
            *alpha_lk_d * (logup_q_evals[0].eval - E::ONE) + *alpha_lk_n * logup_p_evals[0].eval;
        let sel_subclaim = IOPVerifierState::verify(
            claim_sum,
            &IOPProof {
                point: vec![], // final claimed point will be derived from sumcheck protocol
                proofs: proof.sel_sumcheck_proofs.clone(),
            },
            &VPAuxInfo {
                max_degree: SEL_DEGREE.max(cs.max_non_lc_degree),
                num_variables: log2_num_instances,
                phantom: PhantomData,
            },
            transcript,
        );
        let (input_opening_point, expected_evaluation) = (
            sel_subclaim.point.iter().map(|c| c.elements).collect_vec(),
            sel_subclaim.expected_evaluation,
        );
        let eq_lk = build_eq_x_r_vec_sequential(&rt_lk[..log2_lk_count]);

        let sel_lk = eq_eval_less_or_equal_than(
            num_instances - 1,
            &rt_lk[log2_lk_count..],
            &input_opening_point,
        );

        let computed_evals = [
            // lookup denominator
            *alpha_lk_d
                * sel_lk
                * ((0..lk_counts_per_instance)
                    .map(|i| proof.lk_d_in_evals[i] * eq_lk[i])
                    .sum::<E>()
                    + (eq_lk[lk_counts_per_instance..].iter().sum::<E>() - E::ONE)),
            *alpha_lk_n
                * sel_lk
                * ((0..lk_counts_per_instance)
                    .map(|i| proof.lk_n_in_evals[i] * eq_lk[i])
                    .sum::<E>()),
        ]
        .iter()
        .sum::<E>();
        if computed_evals != expected_evaluation {
            return Err(ZKVMError::VerifyError(
                "sel evaluation verify failed".into(),
            ));
        }
        // verify records (degree = 1) statement, thus no sumcheck
        if cs
            .lk_table_expressions
            .iter()
            .map(|lk| &lk.values)
            .chain(cs.lk_table_expressions.iter().map(|lk| &lk.multiplicity))
            .zip_eq(
                proof.lk_d_in_evals[..lk_counts_per_instance]
                    .iter()
                    .chain(proof.lk_n_in_evals[..lk_counts_per_instance].iter()),
            )
            .any(|(expr, expected_evals)| {
                eval_by_expr_with_fixed(
                    &proof.fixed_in_evals,
                    &proof.wits_in_evals,
                    challenges,
                    expr,
                ) != *expected_evals
            })
        {
            return Err(ZKVMError::VerifyError(
                "record evaluate != expected_evals".into(),
            ));
        }

        PCS::simple_batch_verify(
            vp,
            circuit_vk.fixed_commit.as_ref().unwrap(),
            &input_opening_point,
            &proof.fixed_in_evals,
            &proof.fixed_opening_proof,
            transcript,
        )
        .map_err(ZKVMError::PCSError)?;
        tracing::debug!(
            "[table {}] verified opening proof for {} fixed polys at {:?}: values = {:?}, commit = {:?}",
            name,
            proof.fixed_in_evals.len(),
            input_opening_point,
            proof.fixed_in_evals,
            circuit_vk.fixed_commit.as_ref().unwrap(),
        );

        PCS::simple_batch_verify(
            vp,
            &proof.wits_commit,
            &input_opening_point,
            &proof.wits_in_evals,
            &proof.wits_opening_proof,
            transcript,
        )
        .map_err(ZKVMError::PCSError)?;
        tracing::debug!(
            "[table {}] verified opening proof for {} polys at {:?}: values = {:?}, commit = {:?}",
            name,
            proof.wits_in_evals.len(),
            input_opening_point,
            proof.wits_in_evals,
            proof.wits_commit
        );

        Ok(input_opening_point)
    }
}

pub struct TowerVerify;

pub type TowerVerifyResult<E> = Result<
    (
        Point<E>,
        Vec<PointAndEval<E>>,
        Vec<PointAndEval<E>>,
        Vec<PointAndEval<E>>,
    ),
    ZKVMError,
>;

impl TowerVerify {
    pub fn verify<E: ExtensionField>(
        prod_out_evals: Vec<Vec<E>>,
        logup_out_evals: Vec<Vec<E>>,
        tower_proofs: &TowerProofs<E>,
        expected_rounds: Vec<usize>,
        num_fanin: usize,
        transcript: &mut Transcript<E>,
    ) -> TowerVerifyResult<E> {
        /*
        println!("\n\n--\nINPUT:");
        println!("prod_spec_size: {}", prod_out_evals.len());
        println!("logup_spec_size: {}", logup_out_evals.len());
        print!("expected_rounds: [ ");
        for r in &expected_rounds {
            print!("{} ", r);
        }
        println!("]");
        print!("evals_contents: [ ");
        // prod out evals
        for t0 in &prod_out_evals {
            for t1 in t0 {
                print!("{} {} ", t1.as_bases()[0].to_canonical_u64(), t1.as_bases()[1].to_canonical_u64());
            }
        }
        // logup out evals
        for t0 in &logup_out_evals {
            for t1 in t0 {
                print!("{} {} ", t1.as_bases()[0].to_canonical_u64(), t1.as_bases()[1].to_canonical_u64());
            }
        }
        // proofs
        for t0 in &tower_proofs.proofs {
            for t1 in t0 {
                for e in &t1.evaluations {
                    print!("{} {} ", e.as_bases()[0].to_canonical_u64(), e.as_bases()[1].to_canonical_u64());
                }
            }
        }
        // prod_specs_eval
        for t0 in &tower_proofs.prod_specs_eval {
            for t1 in t0 {
                for e in t1 {
                    print!("{} {} ", e.as_bases()[0].to_canonical_u64(), e.as_bases()[1].to_canonical_u64());
                }
            }
        }
        // logup_specs_eval
        for t0 in &tower_proofs.logup_specs_eval {
            for t1 in t0 {
                for e in t1 {
                    print!("{} {} ", e.as_bases()[0].to_canonical_u64(), e.as_bases()[1].to_canonical_u64());
                }
            }
        }
        println!("]");
        print!("transcript_state: [ ");
        for f in &transcript.permutation.state {
            print!("{} ", f.to_canonical_u64());
        }
        println!("]");
        println!("\nWITNESS:");
        */

        // XXX to sumcheck batched product argument with logup, we limit num_product_fanin to 2
        // TODO mayber give a better naming?
        assert_eq!(num_fanin, 2);
        let num_prod_spec = prod_out_evals.len();
        let num_logup_spec = logup_out_evals.len();

        let log2_num_fanin = ceil_log2(num_fanin);
        // sanity check
        assert!(num_prod_spec == tower_proofs.prod_spec_size());
        assert!(prod_out_evals.iter().all(|evals| evals.len() == num_fanin));
        assert!(num_logup_spec == tower_proofs.logup_spec_size());
        assert!(logup_out_evals.iter().all(|evals| {
            evals.len() == 4 // [p1, p2, q1, q2]
        }));
        assert_eq!(expected_rounds.len(), num_prod_spec + num_logup_spec);

        let alpha_pows = get_challenge_pows(
            num_prod_spec + num_logup_spec * 2, /* logup occupy 2 sumcheck: numerator and denominator */
            transcript,
        );
        let initial_rt: Point<E> = (0..log2_num_fanin)
            .map(|_| transcript.get_and_append_challenge(b"product_sum").elements)
            .collect_vec();
        // initial_claim = \sum_j alpha^j * out_j[rt]
        // out_j[rt] := (record_{j}[rt])
        // out_j[rt] := (logup_p{j}[rt])
        // out_j[rt] := (logup_q{j}[rt])
        let initial_claim = izip!(prod_out_evals, alpha_pows.iter())
            .map(|(evals, alpha)| {
                // println!("\n\n--\nEVALS: {:?}", evals);
                // println!("INIT_RT: {:?}", initial_rt);
                // println!("EVAL: {:?}", evals.clone().into_mle().evaluate(&initial_rt));
                evals.into_mle().evaluate(&initial_rt) * alpha
            })
            .sum::<E>()
            + izip!(logup_out_evals, alpha_pows[num_prod_spec..].chunks(2))
                .map(|(evals, alpha)| {
                    let (alpha_numerator, alpha_denominator) = (&alpha[0], &alpha[1]);
                    let (p1, p2, q1, q2) = (evals[0], evals[1], evals[2], evals[3]);
                    vec![p1, p2].into_mle().evaluate(&initial_rt) * alpha_numerator
                        + vec![q1, q2].into_mle().evaluate(&initial_rt) * alpha_denominator
                })
                .sum::<E>();

        // evaluation in the tower input layer
        let mut prod_spec_input_layer_eval = vec![PointAndEval::default(); num_prod_spec];
        let mut logup_spec_p_input_layer_eval = vec![PointAndEval::default(); num_logup_spec];
        let mut logup_spec_q_input_layer_eval = vec![PointAndEval::default(); num_logup_spec];

        let expected_max_round = expected_rounds.iter().max().unwrap();

        let (next_rt, _) = (0..(expected_max_round-1)).try_fold(
            (
                PointAndEval {
                    point: initial_rt,
                    eval: initial_claim,
                },
                alpha_pows,
            ),
            |(point_and_eval, alpha_pows), round| {
                let (out_rt, out_claim) = (&point_and_eval.point, &point_and_eval.eval);
                let sumcheck_claim = IOPVerifierState::verify(
                    *out_claim,
                    &IOPProof {
                        point: vec![], // final claimed point will be derived from sumcheck protocol
                        proofs: tower_proofs.proofs[round].clone(),
                    },
                    &VPAuxInfo {
                        max_degree: NUM_FANIN + 1, // + 1 for eq
                        num_variables: (round + 1) * log2_num_fanin,
                        phantom: PhantomData,
                    },
                    transcript,
                );
                tracing::debug!("verified tower proof at layer {}/{}", round + 1, expected_max_round-1);

                // check expected_evaluation
                let rt: Point<E> = sumcheck_claim.point.iter().map(|c| c.elements).collect();
                let mut expected_evaluation: E = (0..num_prod_spec)
                    .zip(alpha_pows.iter())
                    .zip(expected_rounds.iter())
                    .map(|((spec_index, alpha), max_round)| {
                        eq_eval(out_rt, &rt)
                            * alpha
                            * if round < *max_round-1 {tower_proofs.prod_specs_eval[spec_index][round].iter().product()} else {
                                E::ZERO
                            }
                    })
                    .sum::<E>();
                expected_evaluation += (0..num_logup_spec)
                        .zip_eq(alpha_pows[num_prod_spec..].chunks(2))
                        .zip_eq(expected_rounds[num_prod_spec..].iter())
                        .map(|((spec_index, alpha), max_round)| {
                            let (alpha_numerator, alpha_denominator) = (&alpha[0], &alpha[1]);
                            eq_eval(out_rt, &rt) * if round < *max_round-1 {
                                let evals = &tower_proofs.logup_specs_eval[spec_index][round];
                                let (p1, p2, q1, q2) =
                                        (evals[0], evals[1], evals[2], evals[3]);
                                    *alpha_numerator * (p1 * q2 + p2 * q1)
                                        + *alpha_denominator * (q1 * q2)
                            } else {
                                E::ZERO
                            }
                        })
                        .sum::<E>();
                if expected_evaluation != sumcheck_claim.expected_evaluation {
                    return Err(ZKVMError::VerifyError("mismatch tower evaluation".into()));
                }

                // derive single eval
                // rt' = r_merge || rt
                // r_merge.len() == ceil_log2(num_product_fanin)
                let r_merge = (0..log2_num_fanin)
                    .map(|_| transcript.get_and_append_challenge(b"merge").elements)
                    .collect_vec();
                let coeffs = build_eq_x_r_vec_sequential(&r_merge);
                assert_eq!(coeffs.len(), num_fanin);
                let rt_prime = [rt, r_merge].concat();

                // generate next round challenge
                let next_alpha_pows = get_challenge_pows(
                    num_prod_spec + num_logup_spec * 2, // logup occupy 2 sumcheck: numerator and denominator
                    transcript,
                );
                let next_round = round + 1;
                let next_prod_spec_evals = (0..num_prod_spec)
                    .zip(next_alpha_pows.iter())
                    .zip(expected_rounds.iter())
                    .map(|((spec_index, alpha), max_round)| {
                        if round < max_round -1 {
                            // merged evaluation
                            let evals = izip!(
                                tower_proofs.prod_specs_eval[spec_index][round].iter(),
                                coeffs.iter()
                            )
                            .map(|(a, b)| *a * b)
                            .sum::<E>();
                            // this will keep update until round > evaluation
                            prod_spec_input_layer_eval[spec_index] = PointAndEval::new(rt_prime.clone(), evals);
                            if next_round < max_round -1 {
                                *alpha * evals
                            } else {
                                E::ZERO
                            }
                        } else {
                            E::ZERO
                        }
                    })
                    .sum::<E>();
                let next_logup_spec_evals = (0..num_logup_spec)
                    .zip_eq(next_alpha_pows[num_prod_spec..].chunks(2))
                    .zip_eq(expected_rounds[num_prod_spec..].iter())
                    .map(|((spec_index, alpha), max_round)| {
                        if round < max_round -1 {
                            let (alpha_numerator, alpha_denominator) = (&alpha[0], &alpha[1]);
                            // merged evaluation
                            let p_evals = izip!(
                                tower_proofs.logup_specs_eval[spec_index][round][0..2].iter(),
                                coeffs.iter()
                            )
                            .map(|(a, b)| *a * b)
                            .sum::<E>();

                            let q_evals = izip!(
                                tower_proofs.logup_specs_eval[spec_index][round][2..4].iter(),
                                coeffs.iter()
                            )
                            .map(|(a, b)| *a * b)
                            .sum::<E>();

                            // this will keep update until round > evaluation
                            logup_spec_p_input_layer_eval[spec_index] = PointAndEval::new(rt_prime.clone(), p_evals);
                            logup_spec_q_input_layer_eval[spec_index] = PointAndEval::new(rt_prime.clone(), q_evals);

                            if next_round < max_round -1 {
                                *alpha_numerator * p_evals + *alpha_denominator * q_evals
                            } else {
                                E::ZERO
                            }
                        } else {
                            E::ZERO
                        }
                    })
                    .sum::<E>();
                // sum evaluation from different specs
                let next_eval = next_prod_spec_evals + next_logup_spec_evals;
                Ok((PointAndEval {
                    point: rt_prime,
                    eval: next_eval,
                }, next_alpha_pows))
            },
        )?;

        /*
        println!("\nOUTPUT:");
        println!("NEXT_RT: {:?}", next_rt.point);
        println!("PROD_SPEC_ILE: {:?}", prod_spec_input_layer_eval);
        println!("LOGUP_SPEC_P_ILE: {:?}", logup_spec_p_input_layer_eval);
        println!("LOGUP_SPEC_Q_ILE: {:?}", logup_spec_q_input_layer_eval);
        println!("--\n\n");
        */
        Ok((
            next_rt.point,
            prod_spec_input_layer_eval,
            logup_spec_p_input_layer_eval,
            logup_spec_q_input_layer_eval,
        ))
    }
}
