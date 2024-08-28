use std::marker::PhantomData;

use ark_std::iterable::Iterable;
use ff_ext::ExtensionField;

use itertools::{izip, Itertools};
use multilinear_extensions::{
    mle::{IntoMLE, MultilinearExtension},
    util::ceil_log2,
    virtual_poly::{build_eq_x_r_vec_sequential, eq_eval, VPAuxInfo},
};
use sumcheck::structs::{IOPProof, IOPVerifierState};
use transcript::Transcript;

use crate::{
    circuit_builder::Circuit,
    error::ZKVMError,
    scheme::constants::{NUM_FANIN, SEL_DEGREE},
    structs::{Point, PointAndEval, TowerProofs},
    utils::{get_challenge_pows, sel_eval},
};

use super::{constants::MAINCONSTRAIN_SUMCHECK_BATCH_SIZE, utils::eval_by_expr, ZKVMProof};

pub struct ZKVMVerifier<E: ExtensionField> {
    circuit: Circuit<E>,
}

impl<E: ExtensionField> ZKVMVerifier<E> {
    pub fn new(circuit: Circuit<E>) -> Self {
        ZKVMVerifier { circuit }
    }

    /// verify proof and return input opening point
    pub fn verify(
        &self,
        proof: &ZKVMProof<E>,
        transcript: &mut Transcript<E>,
        num_product_fanin: usize,
        _out_evals: &PointAndEval<E>,
        challenges: &[E; 2], // derive challenge from PCS
    ) -> Result<Point<E>, ZKVMError> {
        let (r_counts_per_instance, w_counts_per_instance, lk_counts_per_instance) = (
            self.circuit.r_expressions.len(),
            self.circuit.w_expressions.len(),
            self.circuit.lk_expressions.len(),
        );
        let (log2_r_count, log2_w_count, log2_lk_count) = (
            ceil_log2(r_counts_per_instance),
            ceil_log2(w_counts_per_instance),
            ceil_log2(lk_counts_per_instance),
        );
        let (chip_record_alpha, _) = (challenges[0], challenges[1]);

        let num_instances = proof.num_instances;
        let log2_num_instances = ceil_log2(num_instances);

        // verify and reduce product tower sumcheck
        let tower_proofs = &proof.tower_proof;

        // TODO check rw_set equality across all proofs
        // TODO check logup relation across all proofs

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
                "Lookup table witness p(x) != constant 1",
            ));
        }

        // verify zero statement (degree > 1) + sel sumcheck
        let (rt_r, rt_w, rt_lk): (Vec<E>, Vec<E>, Vec<E>) = (
            record_evals[0].point.clone(),
            record_evals[1].point.clone(),
            logup_q_evals[0].point.clone(),
        );

        let alpha_pow = get_challenge_pows(
            MAINCONSTRAIN_SUMCHECK_BATCH_SIZE + self.circuit.assert_zero_sumcheck_expressions.len(),
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
                max_degree: SEL_DEGREE.max(self.circuit.max_non_lc_degree),
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
            // sel(rt, t) = eq(rt, t) x sel(t)
            (
                eq_eval(&rt_r[log2_r_count..], &input_opening_point)
                    * sel_eval(num_instances, &input_opening_point),
                eq_eval(&rt_w[log2_w_count..], &input_opening_point)
                    * sel_eval(num_instances, &input_opening_point),
                eq_eval(&rt_lk[log2_lk_count..], &input_opening_point)
                    * sel_eval(num_instances, &input_opening_point),
                // only initialize when circuit got non empty assert_zero_sumcheck_expressions
                {
                    let rt_non_lc_sumcheck = rt_tower[..log2_num_instances].to_vec();
                    if !self.circuit.assert_zero_sumcheck_expressions.is_empty() {
                        Some(
                            eq_eval(&rt_non_lc_sumcheck, &input_opening_point)
                                * sel_eval(num_instances, &rt_non_lc_sumcheck),
                        )
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
                    * self
                        .circuit
                        .assert_zero_sumcheck_expressions
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
                "main + sel evaluation verify failed",
            ));
        }
        // verify records (degree = 1) statement, thus no sumcheck
        if self
            .circuit
            .r_expressions
            .iter()
            .chain(self.circuit.w_expressions.iter())
            .chain(self.circuit.lk_expressions.iter())
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
            return Err(ZKVMError::VerifyError("record evaluate != expected_evals"));
        }

        // verify zero expression (degree = 1) statement, thus no sumcheck
        if self
            .circuit
            .assert_zero_expressions
            .iter()
            .any(|expr| eval_by_expr(&proof.wits_in_evals, challenges, expr) != E::ZERO)
        {
            // TODO add me back
            // return Err(ZKVMError::VerifyError("zero expression != 0"));
        }

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
        init_prod_evals: Vec<Vec<E>>,
        init_logup_evals: Vec<Vec<E>>,
        tower_proofs: &TowerProofs<E>,
        expected_rounds: Vec<usize>,
        num_fanin: usize,
        transcript: &mut Transcript<E>,
    ) -> TowerVerifyResult<E> {
        // XXX to sumcheck batched product argument with logup, we limit num_product_fanin to 2
        // TODO mayber give a better naming?
        assert_eq!(num_fanin, 2);
        let num_prod_spec = init_prod_evals.len();
        let num_logup_spec = init_logup_evals.len();

        let log2_num_fanin = ceil_log2(num_fanin);
        // sanity check
        assert!(num_prod_spec == tower_proofs.prod_spec_size());
        assert!(init_prod_evals.iter().all(|evals| evals.len() == num_fanin));
        assert!(num_logup_spec == tower_proofs.logup_spec_size());
        assert!(init_logup_evals.iter().all(|evals| {
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
        let initial_claim = izip!(init_prod_evals, alpha_pows.iter())
            .map(|(evals, alpha)| evals.into_mle().evaluate(&initial_rt) * alpha)
            .sum::<E>()
            + izip!(init_logup_evals, alpha_pows[num_prod_spec..].chunks(2))
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
                        point: vec![], // final claimed point will be derive from sumcheck protocol
                        proofs: tower_proofs.proofs[round].clone(),
                    },
                    &VPAuxInfo {
                        max_degree: NUM_FANIN + 1, // + 1 for eq
                        num_variables: (round + 1) * log2_num_fanin,
                        phantom: PhantomData,
                    },
                    transcript,
                );

                // check expected_evaluation
                let rt: Point<E> = sumcheck_claim.point.iter().map(|c| c.elements).collect();
                let expected_evaluation: E = (0..num_prod_spec)
                    .zip(alpha_pows.iter())
                    .zip(expected_rounds.iter())
                    .map(|((spec_index, alpha), max_round)| {
                        eq_eval(out_rt, &rt)
                            * alpha
                            * if round < *max_round-1 {tower_proofs.prod_specs_eval[spec_index][round].iter().product()} else {
                                E::ZERO
                            }
                    })
                    .sum::<E>()
                    + (0..num_logup_spec)
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
                    return Err(ZKVMError::VerifyError("mismatch tower evaluation"));
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

        Ok((
            next_rt.point,
            prod_spec_input_layer_eval,
            logup_spec_p_input_layer_eval,
            logup_spec_q_input_layer_eval,
        ))
    }
}
