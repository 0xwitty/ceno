use std::{collections::BTreeMap, sync::Arc};

use ark_std::iterable::Iterable;
use ff_ext::ExtensionField;
use itertools::Itertools;
use multilinear_extensions::{
    commutative_op_mle_pair,
    mle::{DenseMultilinearExtension, FieldType, IntoMLE},
    op_mle,
    util::ceil_log2,
    virtual_poly_v2::ArcMultilinearExtension,
};
use rayon::{
    iter::{
        IndexedParallelIterator, IntoParallelIterator, IntoParallelRefIterator,
        IntoParallelRefMutIterator, ParallelIterator,
    },
    prelude::ParallelSliceMut,
};
use simple_frontend::structs::WitnessId;

use crate::{expression::Expression, scheme::constants::MIN_PAR_SIZE};

/// interleaving multiple mles into mles, and num_limbs indicate number of final limbs vector
/// e.g input [[1,2],[3,4],[5,6],[7,8]], num_limbs=2,log2_per_instance_size=3
/// output [[1,3,5,7,0,0,0,0],[2,4,6,8,0,0,0,0]]
pub(crate) fn interleaving_mles_to_mles<'a, E: ExtensionField>(
    mles: &[ArcMultilinearExtension<E>],
    log2_num_instances: usize,
    log2_per_instance_size: usize,
    num_limbs: usize,
    default: E,
) -> Vec<ArcMultilinearExtension<'a, E>> {
    let num_instances = 1 << log2_num_instances;
    assert!(num_limbs.is_power_of_two() && log2_per_instance_size.is_power_of_two());
    assert!(!mles.is_empty());
    assert!(
        mles.iter()
            .all(|mle| mle.evaluations().len() == num_instances)
    );
    let per_fanin_len = mles[0].evaluations().len() / num_limbs;
    let log_num_limbs = ceil_log2(num_limbs);

    (0..num_limbs)
        .into_par_iter()
        .map(|fanin_index| {
            let mut evaluations =
                vec![default; 1 << (log2_num_instances + log2_per_instance_size - log_num_limbs)];
            let per_instance_size = 1 << log2_per_instance_size;
            let start = per_fanin_len * fanin_index;
            mles.iter()
                .enumerate()
                .for_each(|(i, mle)| match mle.evaluations() {
                    FieldType::Ext(mle) => mle[start..][..per_fanin_len]
                        .par_iter()
                        .zip_eq(evaluations.par_chunks_mut(per_instance_size))
                        .with_min_len(MIN_PAR_SIZE)
                        .for_each(|(value, instance)| {
                            assert_eq!(instance.len(), per_instance_size);
                            instance[i] = *value;
                        }),
                    _ => {
                        unreachable!("must be extension field")
                    }
                });
            evaluations.into_mle().into()
        })
        .collect::<Vec<ArcMultilinearExtension<E>>>()
}

/// infer logup witness from last layer
/// return is the ([p1,p2], [q1,q2]) for each layer
pub(crate) fn infer_tower_logup_witness<'a, E: ExtensionField>(
    q_mles: Vec<ArcMultilinearExtension<'a, E>>,
) -> Vec<Vec<ArcMultilinearExtension<'a, E>>> {
    if cfg!(test) {
        assert_eq!(q_mles.len(), 2);
        assert!(q_mles.iter().map(|q| q.evaluations().len()).all_equal());
    }
    let num_vars = ceil_log2(q_mles[0].evaluations().len());
    let mut r_wit_layers = (0..num_vars).fold(
        vec![(Option::<Vec<ArcMultilinearExtension<E>>>::None, q_mles)],
        |mut acc, _| {
            let (p, q): &(
                Option<Vec<ArcMultilinearExtension<E>>>,
                Vec<ArcMultilinearExtension<E>>,
            ) = acc.last().unwrap();
            let (q1, q2) = (&q[0], &q[1]);
            let cur_len = q1.evaluations().len() / 2;
            let (next_p, next_q): (
                Vec<ArcMultilinearExtension<E>>,
                Vec<ArcMultilinearExtension<E>>,
            ) = (0..2)
                .map(|index| {
                    let mut p_evals = vec![E::ZERO; cur_len];
                    let mut q_evals = vec![E::ZERO; cur_len];
                    let start_index = cur_len * index;
                    if let Some(p) = p {
                        let (p1, p2) = (&p[0], &p[1]);
                        match (
                            p1.evaluations(),
                            p2.evaluations(),
                            q1.evaluations(),
                            q2.evaluations(),
                        ) {
                            (
                                FieldType::Ext(p1),
                                FieldType::Ext(p2),
                                FieldType::Ext(q1),
                                FieldType::Ext(q2),
                            ) => q1[start_index..][..cur_len]
                                .par_iter()
                                .zip(q2[start_index..][..cur_len].par_iter())
                                .zip(p1[start_index..][..cur_len].par_iter())
                                .zip(p2[start_index..][..cur_len].par_iter())
                                .zip(p_evals.par_iter_mut())
                                .zip(q_evals.par_iter_mut())
                                .with_min_len(MIN_PAR_SIZE)
                                .for_each(|(((((q1, q2), p1), p2), p_eval), q_eval)| {
                                    *p_eval = *p2 * q1 + *p1 * q2;
                                    *q_eval = *q1 * q2;
                                }),
                            _ => unreachable!(),
                        };
                    } else {
                        match (q1.evaluations(), q2.evaluations()) {
                            (FieldType::Ext(q1), FieldType::Ext(q2)) => q1[start_index..]
                                [..cur_len]
                                .par_iter()
                                .zip(q2[start_index..][..cur_len].par_iter())
                                .zip(p_evals.par_iter_mut())
                                .zip(q_evals.par_iter_mut())
                                .with_min_len(MIN_PAR_SIZE)
                                .for_each(|(((q1, q2), p_res), q_res)| {
                                    *p_res = *q1 + q2;
                                    *q_res = *q1 * q2
                                }),
                            _ => unreachable!(),
                        };
                    }
                    (p_evals.into_mle().into(), q_evals.into_mle().into())
                })
                .unzip(); // vec[vec[p1, p2], vec[q1, q2]]
            acc.push((Some(next_p), next_q));
            acc
        },
    );
    r_wit_layers.reverse();
    r_wit_layers
        .into_iter()
        .map(|(p, q)| {
            // input layer p are all 1
            if p.is_none() {
                let len = q[0].evaluations().len();
                vec![
                    vec![E::ONE; len].into_mle().into(),
                    vec![E::ONE; len].into_mle().into(),
                ]
                .into_iter()
                .chain(q.into_iter())
                .collect()
            } else {
                vec![p.unwrap(), q].concat()
            }
        })
        .collect_vec()
}

/// infer tower witness from last layer
pub(crate) fn infer_tower_product_witness<'a, E: ExtensionField>(
    num_vars: usize,
    last_layer: Vec<ArcMultilinearExtension<'a, E>>,
    num_product_fanin: usize,
) -> Vec<Vec<ArcMultilinearExtension<'a, E>>> {
    assert!(last_layer.len() == num_product_fanin);
    let mut r_wit_layers = (0..num_vars - 1).fold(vec![last_layer], |mut acc, _| {
        let next_layer = acc.last().unwrap();
        let cur_len = next_layer[0].evaluations().len() / num_product_fanin;
        let cur_layer: Vec<ArcMultilinearExtension<E>> = (0..num_product_fanin)
            .map(|index| {
                let mut evaluations = vec![E::ONE; cur_len];
                next_layer.iter().for_each(|f| match f.evaluations() {
                    FieldType::Ext(f) => {
                        let start: usize = index * cur_len;
                        f[start..][..cur_len]
                            .par_iter()
                            .zip(evaluations.par_iter_mut())
                            .with_min_len(MIN_PAR_SIZE)
                            .map(|(v, evaluations)| *evaluations *= *v)
                            .collect()
                    }
                    _ => unreachable!("must be extension field"),
                });
                evaluations.into_mle().into()
            })
            .collect_vec();
        acc.push(cur_layer);
        acc
    });
    r_wit_layers.reverse();
    r_wit_layers
}

pub(crate) fn wit_infer_by_expr<'a, E: ExtensionField>(
    witnesses: &BTreeMap<WitnessId, DenseMultilinearExtension<E>>,
    challenges: &[E],
    expr: &Expression<E>,
) -> ArcMultilinearExtension<'a, E> {
    expr.evaluate::<ArcMultilinearExtension<'_, E>>(
        &|witness_id| {
            let a: ArcMultilinearExtension<E> = Arc::new(
                witnesses
                    .get(&witness_id)
                    .expect("non exist witness")
                    .clone(),
            );
            a
        },
        &|scalar| {
            let scalar: ArcMultilinearExtension<E> = Arc::new(
                DenseMultilinearExtension::from_evaluations_vec(0, vec![scalar]),
            );
            scalar
        },
        &|challenge_id, pow, scalar, offset| {
            // TODO cache challenge power to be aquire once for each power
            let challenge = challenges[challenge_id as usize];
            let challenge: ArcMultilinearExtension<E> =
                Arc::new(DenseMultilinearExtension::from_evaluations_ext_vec(
                    0,
                    vec![challenge.pow(&[pow as u64]) * scalar + offset],
                ));
            challenge
        },
        &|a, b| {
            commutative_op_mle_pair!(|a, b| {
                match (a.len(), b.len()) {
                    (1, 1) => Arc::new(DenseMultilinearExtension::from_evaluation_vec_smart(
                        0,
                        vec![a[0] + b[0]],
                    )),
                    (1, _) => Arc::new(DenseMultilinearExtension::from_evaluation_vec_smart(
                        ceil_log2(b.len()),
                        b.par_iter()
                            .with_min_len(MIN_PAR_SIZE)
                            .map(|b| a[0] + *b)
                            .collect(),
                    )),
                    (_, 1) => Arc::new(DenseMultilinearExtension::from_evaluation_vec_smart(
                        ceil_log2(a.len()),
                        a.par_iter()
                            .with_min_len(MIN_PAR_SIZE)
                            .map(|a| *a + b[0])
                            .collect(),
                    )),
                    (_, _) => Arc::new(DenseMultilinearExtension::from_evaluation_vec_smart(
                        ceil_log2(a.len()),
                        a.par_iter()
                            .zip(b.par_iter())
                            .with_min_len(MIN_PAR_SIZE)
                            .map(|(a, b)| *a + b)
                            .collect(),
                    )),
                }
            })
        },
        &|a, b| {
            commutative_op_mle_pair!(|a, b| {
                match (a.len(), b.len()) {
                    (1, 1) => Arc::new(DenseMultilinearExtension::from_evaluation_vec_smart(
                        0,
                        vec![a[0] * b[0]],
                    )),
                    (1, _) => Arc::new(DenseMultilinearExtension::from_evaluation_vec_smart(
                        ceil_log2(b.len()),
                        b.par_iter()
                            .with_min_len(MIN_PAR_SIZE)
                            .map(|b| a[0] * *b)
                            .collect(),
                    )),
                    (_, 1) => Arc::new(DenseMultilinearExtension::from_evaluation_vec_smart(
                        ceil_log2(a.len()),
                        a.par_iter()
                            .with_min_len(MIN_PAR_SIZE)
                            .map(|a| *a * b[0])
                            .collect(),
                    )),
                    (_, _) => {
                        unimplemented!("r,w only support degree 1 expression")
                    }
                }
            })
        },
        &|x, a, b| {
            let a = op_mle!(
                |a| {
                    assert_eq!(a.len(), 1);
                    a[0]
                },
                |a| a.into()
            );
            let b = op_mle!(
                |b| {
                    assert_eq!(b.len(), 1);
                    b[0]
                },
                |b| b.into()
            );
            op_mle!(|x| {
                Arc::new(DenseMultilinearExtension::from_evaluation_vec_smart(
                    ceil_log2(x.len()),
                    x.par_iter()
                        .with_min_len(MIN_PAR_SIZE)
                        .map(|x| a * x + b)
                        .collect(),
                ))
            })
        },
    )
}

pub(crate) fn eval_by_expr<'a, E: ExtensionField>(
    witnesses: &[E],
    challenges: &[E],
    expr: &Expression<E>,
) -> E {
    expr.evaluate::<E>(
        &|witness_id| witnesses[witness_id as usize],
        &|scalar| scalar.into(),
        &|challenge_id, pow, scalar, offset| {
            // TODO cache challenge power to be aquire once for each power
            let challenge = challenges[challenge_id as usize];
            challenge.pow(&[pow as u64]) * scalar + offset
        },
        &|a, b| a + b,
        &|a, b| a * b,
        &|x, a, b| a * x + b,
    )
}

#[cfg(test)]
mod tests {
    use ff::Field;
    use goldilocks::{ExtensionField, GoldilocksExt2};
    use itertools::Itertools;
    use multilinear_extensions::{
        commutative_op_mle_pair,
        mle::{FieldType, IntoMLE},
        util::ceil_log2,
        virtual_poly_v2::ArcMultilinearExtension,
    };

    use crate::scheme::utils::{
        infer_tower_logup_witness, infer_tower_product_witness, interleaving_mles_to_mles,
    };

    #[test]
    fn test_infer_tower_witness() {
        type E = GoldilocksExt2;
        let num_product_fanin = 2;
        let last_layer: Vec<ArcMultilinearExtension<E>> = vec![
            vec![E::ONE, E::from(2u64)].into_mle().into(),
            vec![E::from(3u64), E::from(4u64)].into_mle().into(),
        ];
        let num_vars = ceil_log2(last_layer[0].evaluations().len()) + 1;
        let res = infer_tower_product_witness(num_vars, last_layer.clone(), 2);
        let (left, right) = (&res[0][0], &res[0][1]);
        let final_product = commutative_op_mle_pair!(
            |left, right| {
                assert!(left.len() == 1 && right.len() == 1);
                left[0] * right[0]
            },
            |out| E::from_base(&out)
        );
        let expected_final_product: E = last_layer
            .iter()
            .map(|f| match f.evaluations() {
                FieldType::Ext(e) => e.iter().cloned().reduce(|a, b| a * b).unwrap(),
                _ => unreachable!(""),
            })
            .product();
        assert_eq!(res.len(), num_vars);
        assert!(
            res.iter()
                .all(|layer_wit| layer_wit.len() == num_product_fanin)
        );
        assert_eq!(final_product, expected_final_product);
    }

    #[test]
    fn test_interleaving_mles_to_mles() {
        type E = GoldilocksExt2;
        let num_product_fanin = 2;
        // [[1, 2], [3, 4], [5, 6], [7, 8]]
        let input_mles: Vec<ArcMultilinearExtension<E>> = vec![
            vec![E::ONE, E::from(2u64)].into_mle().into(),
            vec![E::from(3u64), E::from(4u64)].into_mle().into(),
            vec![E::from(5u64), E::from(6u64)].into_mle().into(),
            vec![E::from(7u64), E::from(8u64)].into_mle().into(),
        ];
        let res = interleaving_mles_to_mles(&input_mles, 1, 2, num_product_fanin, E::ONE);
        // [[1, 3, 5, 7], [2, 4, 6, 8]]
        assert_eq!(
            res[0].get_ext_field_vec(),
            vec![E::ONE, E::from(3u64), E::from(5u64), E::from(7u64)],
        );
        assert_eq!(
            res[1].get_ext_field_vec(),
            vec![E::from(2u64), E::from(4u64), E::from(6u64), E::from(8u64)],
        );
    }

    #[test]
    fn test_interleaving_mles_to_mles_padding() {
        type E = GoldilocksExt2;
        let num_product_fanin = 2;
        // [[1,2],[3,4],[5,6]]]
        let input_mles: Vec<ArcMultilinearExtension<E>> = vec![
            vec![E::ONE, E::from(2u64)].into_mle().into(),
            vec![E::from(3u64), E::from(4u64)].into_mle().into(),
            vec![E::from(5u64), E::from(6u64)].into_mle().into(),
        ];
        let res = interleaving_mles_to_mles(&input_mles, 1, 2, num_product_fanin, E::ZERO);
        // [[1, 3, 5, 0], [2, 4, 6, 0]]
        assert_eq!(
            res[0].get_ext_field_vec(),
            vec![E::ONE, E::from(3u64), E::from(5u64), E::from(0u64)],
        );
        assert_eq!(
            res[1].get_ext_field_vec(),
            vec![E::from(2u64), E::from(4u64), E::from(6u64), E::from(0u64)],
        );
    }

    #[test]
    fn test_infer_tower_logup_witness() {
        type E = GoldilocksExt2;
        let num_vars = 2;
        let q: Vec<ArcMultilinearExtension<E>> = vec![
            vec![1, 2, 3, 4]
                .into_iter()
                .map(E::from)
                .collect_vec()
                .into_mle()
                .into(),
            vec![5, 6, 7, 8]
                .into_iter()
                .map(E::from)
                .collect_vec()
                .into_mle()
                .into(),
        ];
        let mut res = infer_tower_logup_witness(q.try_into().unwrap());
        assert_eq!(num_vars + 1, res.len());
        // input layer
        let layer = res.pop().unwrap();
        // input layer p
        assert_eq!(
            layer[0].evaluations().clone(),
            FieldType::Ext(vec![1.into(); 4])
        );
        assert_eq!(
            layer[1].evaluations().clone(),
            FieldType::Ext(vec![1.into(); 4])
        );
        // input layer q is none
        assert_eq!(
            layer[2].evaluations().clone(),
            FieldType::Ext(vec![1.into(), 2.into(), 3.into(), 4.into()])
        );
        assert_eq!(
            layer[3].evaluations().clone(),
            FieldType::Ext(vec![5.into(), 6.into(), 7.into(), 8.into()])
        );

        // next layer
        let layer = res.pop().unwrap();
        // next layer p1
        assert_eq!(
            layer[0].evaluations().clone(),
            FieldType::<E>::Ext(vec![
                vec![1 * 1 + 5 * 1].into_iter().map(E::from).sum::<E>(),
                vec![2 * 1 + 6 * 1].into_iter().map(E::from).sum::<E>()
            ])
        );
        // next layer p2
        assert_eq!(
            layer[1].evaluations().clone(),
            FieldType::<E>::Ext(vec![
                vec![3 * 1 + 7 * 1].into_iter().map(E::from).sum::<E>(),
                vec![4 * 1 + 8 * 1].into_iter().map(E::from).sum::<E>()
            ])
        );
        // next layer q1
        assert_eq!(
            layer[2].evaluations().clone(),
            FieldType::<E>::Ext(vec![
                vec![1 * 5].into_iter().map(E::from).sum::<E>(),
                vec![2 * 6].into_iter().map(E::from).sum::<E>()
            ])
        );
        // next layer q2
        assert_eq!(
            layer[3].evaluations().clone(),
            FieldType::<E>::Ext(vec![
                vec![3 * 7].into_iter().map(E::from).sum::<E>(),
                vec![4 * 8].into_iter().map(E::from).sum::<E>()
            ])
        );

        // output layer
        let layer = res.pop().unwrap();
        // p1
        assert_eq!(
            layer[0].evaluations().clone(),
            // p11 * q12 + p12 * q11
            FieldType::<E>::Ext(vec![
                vec![(1 * 1 + 5 * 1) * (3 * 7) + (3 * 1 + 7 * 1) * (1 * 5)]
                    .into_iter()
                    .map(E::from)
                    .sum::<E>(),
            ])
        );
        // p2
        assert_eq!(
            layer[1].evaluations().clone(),
            // p21 * q22 + p22 * q21
            FieldType::<E>::Ext(vec![
                vec![(2 * 1 + 6 * 1) * (4 * 8) + (4 * 1 + 8 * 1) * (2 * 6)]
                    .into_iter()
                    .map(E::from)
                    .sum::<E>(),
            ])
        );
        // q1
        assert_eq!(
            layer[2].evaluations().clone(),
            // q12 * q11
            FieldType::<E>::Ext(vec![
                vec![(3 * 7) * (1 * 5)].into_iter().map(E::from).sum::<E>(),
            ])
        );
        // q2
        assert_eq!(
            layer[3].evaluations().clone(),
            // q22 * q22
            FieldType::<E>::Ext(vec![
                vec![(4 * 8) * (2 * 6)].into_iter().map(E::from).sum::<E>(),
            ])
        );
    }
}