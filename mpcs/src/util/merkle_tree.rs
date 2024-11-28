use ff_ext::ExtensionField;
use itertools::Itertools;
use multilinear_extensions::mle::FieldType;
use rayon::{
    iter::{
        IndexedParallelIterator, IntoParallelIterator, IntoParallelRefMutIterator, ParallelIterator,
    },
    slice::ParallelSlice,
};

use crate::util::{
    Deserialize, DeserializeOwned, Serialize, field_type_index_base, field_type_index_ext,
    hash::{
        Digest, hash_two_digests, hash_two_leaves_base, hash_two_leaves_batch_base,
        hash_two_leaves_batch_ext, hash_two_leaves_ext,
    },
    log2_strict,
};
use transcript::Transcript;

use ark_std::{end_timer, start_timer};

use super::hash::write_digest_to_transcript;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(bound(deserialize = "E: DeserializeOwned"))]
pub struct MerkleTreeDigests<E: ExtensionField>
where
    E::BaseField: Serialize + DeserializeOwned,
{
    // This structure contains all the digests in the Merkle tree without
    // the leaves. Each vector represents a layer in the Merkle tree.
    // The first vector consists of the parents of the leaves, so the
    // size of the first vector is `merkle_tree_size / 2`.
    // The last vector consists of only the root.
    // The length of the outer vector is exactly the Merkle tree height.
    inner: Vec<Vec<Digest<E::BaseField>>>,
}

impl<E: ExtensionField> MerkleTreeDigests<E>
where
    E::BaseField: Serialize + DeserializeOwned,
{
    pub fn from_leaves(leaves: &FieldType<E>) -> Self {
        Self {
            inner: merkelize::<E>(&[leaves]),
        }
    }
    pub fn from_leaves_base(leaves: &[E::BaseField]) -> Self {
        Self {
            inner: merkelize_base::<E>(&[leaves]),
        }
    }
    pub fn from_leaves_ext(leaves: &[E]) -> Self {
        Self {
            inner: merkelize_ext::<E>(&[leaves]),
        }
    }

    pub fn from_batch_leaves(leaves: &[&FieldType<E>]) -> Self {
        Self {
            inner: merkelize::<E>(leaves),
        }
    }

    pub fn root(&self) -> Digest<E::BaseField> {
        self.inner.last().unwrap()[0].clone()
    }

    pub fn root_ref(&self) -> &Digest<E::BaseField> {
        &self.inner.last().unwrap()[0]
    }

    pub fn height(&self) -> usize {
        self.inner.len()
    }

    pub fn bottom_size(&self) -> usize {
        self.inner.first().unwrap().len()
    }

    // Given the leaf group index, returns the Merkle path for this
    // leaf group. Here a leaf group represents two leaves that
    // are hashed together in the tree. The leaf group index is
    // the index of this leaf group in all the leaf groups.
    pub fn merkle_path_without_leaf_sibling_or_root(
        &self,
        leaf_group_index: usize,
    ) -> MerklePathWithoutLeafOrRoot<E> {
        assert!(leaf_group_index < self.bottom_size());
        MerklePathWithoutLeafOrRoot::new(
            self.inner
                .iter()
                .take(self.height() - 1)
                .enumerate()
                .map(|(layer_index, layer)| {
                    // For each leaf group (i.e., 2 leaves that are hashed
                    // together into their parent node), their ancestors
                    // consist of their parent node, and all the way
                    // up to the root.
                    // Their parent is in `layer[0]`, and the index of their
                    // parent in `layer[0]` is exactly `leaf_group_index`.
                    // Similarly, the grandparent is in `layer[1]`, then the
                    // index of their grandparent in `layer[1]` is exactly
                    // `leaf_group_index >> 1`. In general, their ancestors
                    // are
                    // `layers[layer_index][(leaf_group_index >> layer_index)]`.
                    // Note that the Merkle path are not the ancestors, but
                    // siblings of all the ancestors, that's why we need the
                    // `^1`.
                    Digest::<E::BaseField>(layer[(leaf_group_index >> layer_index) ^ 1].clone().0)
                })
                .collect(),
        )
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(bound(deserialize = "E: DeserializeOwned"))]
pub struct MerkleTree<E: ExtensionField>
where
    E::BaseField: Serialize + DeserializeOwned,
{
    inner: MerkleTreeDigests<E>,
    leaves: Vec<FieldType<E>>,
}

impl<E: ExtensionField> MerkleTree<E>
where
    E::BaseField: Serialize + DeserializeOwned,
{
    pub fn new(inner: MerkleTreeDigests<E>, leaves: FieldType<E>) -> Self {
        Self {
            inner,
            leaves: vec![leaves],
        }
    }

    pub fn from_leaves(leaves: FieldType<E>) -> Self {
        Self {
            inner: MerkleTreeDigests::<E>::from_leaves(&leaves),
            leaves: vec![leaves],
        }
    }

    pub fn from_batch_leaves(leaves: Vec<FieldType<E>>) -> Self {
        Self {
            inner: MerkleTreeDigests::<E>::from_batch_leaves(&leaves.iter().collect_vec()),
            leaves,
        }
    }

    pub fn root(&self) -> Digest<E::BaseField> {
        self.inner.root()
    }

    pub fn root_ref(&self) -> &Digest<E::BaseField> {
        self.inner.root_ref()
    }

    pub fn height(&self) -> usize {
        self.inner.height()
    }

    pub fn leaves(&self) -> &Vec<FieldType<E>> {
        &self.leaves
    }

    pub fn batch_leaves(&self, coeffs: &[E]) -> Vec<E> {
        (0..self.leaves[0].len())
            .into_par_iter()
            .map(|i| self.batch_leaf(coeffs, i))
            .collect()
    }

    pub fn batch_leaf(&self, coeffs: &[E], index: usize) -> E {
        self.leaves
            .iter()
            .zip(coeffs.iter())
            .map(|(leaf, coeff)| field_type_index_ext(leaf, index) * *coeff)
            .sum()
    }

    pub fn leaves_size(&self) -> (usize, usize) {
        (self.leaves.len(), self.leaves[0].len())
    }

    pub fn get_leaf_as_base(&self, index: usize) -> Vec<E::BaseField> {
        match &self.leaves[0] {
            FieldType::Base(_) => self
                .leaves
                .iter()
                .map(|leaves| field_type_index_base(leaves, index))
                .collect(),
            FieldType::Ext(_) => panic!(
                "Mismatching field type, calling get_leaf_as_base on a Merkle tree over extension fields"
            ),
            FieldType::Unreachable => unreachable!(),
        }
    }

    pub fn get_leaf_as_extension(&self, index: usize) -> Vec<E> {
        match &self.leaves[0] {
            FieldType::Base(_) => self
                .leaves
                .iter()
                .map(|leaves| field_type_index_ext(leaves, index))
                .collect(),
            FieldType::Ext(_) => self
                .leaves
                .iter()
                .map(|leaves| field_type_index_ext(leaves, index))
                .collect(),
            FieldType::Unreachable => unreachable!(),
        }
    }

    pub fn merkle_path_without_leaf_sibling_or_root(
        &self,
        leaf_index: usize,
    ) -> MerklePathWithoutLeafOrRoot<E> {
        assert!(leaf_index < self.leaves_size().1);
        // The inner (i.e., the digests without leaves) have half
        // number of bottom-layer nodes than the Merkle tree leaves.
        // So leaves of index 2i and 2i+1 in Merkle tree corresponds
        // to the index i in the inner.
        self.inner
            .merkle_path_without_leaf_sibling_or_root(leaf_index >> 1)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MerklePathWithoutLeafOrRoot<E: ExtensionField>
where
    E::BaseField: Serialize + DeserializeOwned,
{
    inner: Vec<Digest<E::BaseField>>,
}

impl<E: ExtensionField> MerklePathWithoutLeafOrRoot<E>
where
    E::BaseField: Serialize + DeserializeOwned,
{
    pub fn new(inner: Vec<Digest<E::BaseField>>) -> Self {
        Self { inner }
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Digest<E::BaseField>> {
        self.inner.iter()
    }

    pub fn write_transcript(&self, transcript: &mut Transcript<E>) {
        self.inner
            .iter()
            .for_each(|hash| write_digest_to_transcript(hash, transcript));
    }

    pub fn authenticate_leaves_root_ext(
        &self,
        left: E,
        right: E,
        index: usize,
        root: &Digest<E::BaseField>,
    ) {
        authenticate_merkle_path_root::<E>(
            &self.inner,
            FieldType::Ext(vec![left, right]),
            index,
            root,
        )
    }

    pub fn authenticate_leaves_root_base(
        &self,
        left: E::BaseField,
        right: E::BaseField,
        index: usize,
        root: &Digest<E::BaseField>,
    ) {
        authenticate_merkle_path_root::<E>(
            &self.inner,
            FieldType::Base(vec![left, right]),
            index,
            root,
        )
    }

    pub fn authenticate_batch_leaves_root_ext(
        &self,
        left: Vec<E>,
        right: Vec<E>,
        index: usize,
        root: &Digest<E::BaseField>,
    ) {
        authenticate_merkle_path_root_batch::<E>(
            &self.inner,
            FieldType::Ext(left),
            FieldType::Ext(right),
            index,
            root,
        )
    }

    pub fn authenticate_batch_leaves_root_base(
        &self,
        left: Vec<E::BaseField>,
        right: Vec<E::BaseField>,
        index: usize,
        root: &Digest<E::BaseField>,
    ) {
        authenticate_merkle_path_root_batch::<E>(
            &self.inner,
            FieldType::Base(left),
            FieldType::Base(right),
            index,
            root,
        )
    }
}

/// Merkle tree construction
/// TODO: Support merkelizing mixed-type values
fn merkelize<E: ExtensionField>(values: &[&FieldType<E>]) -> Vec<Vec<Digest<E::BaseField>>> {
    #[cfg(feature = "sanity-check")]
    for i in 0..(values.len() - 1) {
        assert_eq!(values[i].len(), values[i + 1].len());
    }
    let timer = start_timer!(|| format!("merkelize {} values", values[0].len() * values.len()));
    let log_v = log2_strict(values[0].len());
    let mut tree = Vec::with_capacity(log_v);
    // The first layer of hashes, half the number of leaves
    let mut hashes = vec![Digest::default(); values[0].len() >> 1];
    if values.len() == 1 {
        hashes.par_iter_mut().enumerate().for_each(|(i, hash)| {
            *hash = match &values[0] {
                FieldType::Base(values) => {
                    hash_two_leaves_base::<E>(&values[i << 1], &values[(i << 1) + 1])
                }
                FieldType::Ext(values) => {
                    hash_two_leaves_ext::<E>(&values[i << 1], &values[(i << 1) + 1])
                }
                FieldType::Unreachable => unreachable!(),
            };
        });
    } else {
        hashes.par_iter_mut().enumerate().for_each(|(i, hash)| {
            *hash = match &values[0] {
                FieldType::Base(_) => hash_two_leaves_batch_base::<E>(
                    values
                        .iter()
                        .map(|values| field_type_index_base(values, i << 1))
                        .collect_vec()
                        .as_slice(),
                    values
                        .iter()
                        .map(|values| field_type_index_base(values, (i << 1) + 1))
                        .collect_vec()
                        .as_slice(),
                ),
                FieldType::Ext(_) => hash_two_leaves_batch_ext::<E>(
                    values
                        .iter()
                        .map(|values| field_type_index_ext(values, i << 1))
                        .collect_vec()
                        .as_slice(),
                    values
                        .iter()
                        .map(|values| field_type_index_ext(values, (i << 1) + 1))
                        .collect_vec()
                        .as_slice(),
                ),
                FieldType::Unreachable => unreachable!(),
            };
        });
    }

    tree.push(hashes);

    for i in 1..(log_v) {
        let oracle = tree[i - 1]
            .par_chunks_exact(2)
            .map(|ys| hash_two_digests(&ys[0], &ys[1]))
            .collect::<Vec<_>>();

        tree.push(oracle);
    }
    end_timer!(timer);
    tree
}

fn merkelize_base<E: ExtensionField>(values: &[&[E::BaseField]]) -> Vec<Vec<Digest<E::BaseField>>> {
    #[cfg(feature = "sanity-check")]
    for i in 0..(values.len() - 1) {
        assert_eq!(values[i].len(), values[i + 1].len());
    }
    let timer = start_timer!(|| format!("merkelize {} values", values[0].len() * values.len()));
    let log_v = log2_strict(values[0].len());
    let mut tree = Vec::with_capacity(log_v);
    // The first layer of hashes, half the number of leaves
    let mut hashes = vec![Digest::default(); values[0].len() >> 1];
    if values.len() == 1 {
        hashes.par_iter_mut().enumerate().for_each(|(i, hash)| {
            *hash = hash_two_leaves_base::<E>(&values[0][i << 1], &values[0][(i << 1) + 1]);
        });
    } else {
        hashes.par_iter_mut().enumerate().for_each(|(i, hash)| {
            *hash = hash_two_leaves_batch_base::<E>(
                values
                    .iter()
                    .map(|values| values[i << 1])
                    .collect_vec()
                    .as_slice(),
                values
                    .iter()
                    .map(|values| values[(i << 1) + 1])
                    .collect_vec()
                    .as_slice(),
            );
        });
    }

    tree.push(hashes);

    for i in 1..(log_v) {
        let oracle = tree[i - 1]
            .par_chunks_exact(2)
            .map(|ys| hash_two_digests(&ys[0], &ys[1]))
            .collect::<Vec<_>>();

        tree.push(oracle);
    }
    end_timer!(timer);
    tree
}

fn merkelize_ext<E: ExtensionField>(values: &[&[E]]) -> Vec<Vec<Digest<E::BaseField>>> {
    #[cfg(feature = "sanity-check")]
    for i in 0..(values.len() - 1) {
        assert_eq!(values[i].len(), values[i + 1].len());
    }
    let timer = start_timer!(|| format!("merkelize {} values", values[0].len() * values.len()));
    let log_v = log2_strict(values[0].len());
    let mut tree = Vec::with_capacity(log_v);
    // The first layer of hashes, half the number of leaves
    let mut hashes = vec![Digest::default(); values[0].len() >> 1];
    if values.len() == 1 {
        hashes.par_iter_mut().enumerate().for_each(|(i, hash)| {
            *hash = hash_two_leaves_ext::<E>(&values[0][i << 1], &values[0][(i << 1) + 1]);
        });
    } else {
        hashes.par_iter_mut().enumerate().for_each(|(i, hash)| {
            *hash = hash_two_leaves_batch_ext::<E>(
                values
                    .iter()
                    .map(|values| values[i << 1])
                    .collect_vec()
                    .as_slice(),
                values
                    .iter()
                    .map(|values| values[(i << 1) + 1])
                    .collect_vec()
                    .as_slice(),
            );
        });
    }

    tree.push(hashes);

    for i in 1..(log_v) {
        let oracle = tree[i - 1]
            .par_chunks_exact(2)
            .map(|ys| hash_two_digests(&ys[0], &ys[1]))
            .collect::<Vec<_>>();

        tree.push(oracle);
    }
    end_timer!(timer);
    tree
}

fn authenticate_merkle_path_root<E: ExtensionField>(
    path: &[Digest<E::BaseField>],
    leaves: FieldType<E>,
    x_index: usize,
    root: &Digest<E::BaseField>,
) {
    let mut x_index = x_index;
    assert_eq!(leaves.len(), 2);
    let mut hash = match leaves {
        FieldType::Base(leaves) => hash_two_leaves_base::<E>(&leaves[0], &leaves[1]),
        FieldType::Ext(leaves) => hash_two_leaves_ext(&leaves[0], &leaves[1]),
        FieldType::Unreachable => unreachable!(),
    };

    // The lowest bit in the index is ignored. It can point to either leaves
    x_index >>= 1;
    for path_i in path.iter() {
        hash = if x_index & 1 == 0 {
            hash_two_digests(&hash, path_i)
        } else {
            hash_two_digests(path_i, &hash)
        };
        x_index >>= 1;
    }
    assert_eq!(&hash, root);
}

fn authenticate_merkle_path_root_batch<E: ExtensionField>(
    path: &[Digest<E::BaseField>],
    left: FieldType<E>,
    right: FieldType<E>,
    x_index: usize,
    root: &Digest<E::BaseField>,
) {
    let mut x_index = x_index;
    let mut hash = if left.len() > 1 {
        match (left, right) {
            (FieldType::Base(left), FieldType::Base(right)) => {
                hash_two_leaves_batch_base::<E>(&left, &right)
            }
            (FieldType::Ext(left), FieldType::Ext(right)) => {
                hash_two_leaves_batch_ext::<E>(&left, &right)
            }
            _ => unreachable!(),
        }
    } else {
        match (left, right) {
            (FieldType::Base(left), FieldType::Base(right)) => {
                hash_two_leaves_base::<E>(&left[0], &right[0])
            }
            (FieldType::Ext(left), FieldType::Ext(right)) => {
                hash_two_leaves_ext::<E>(&left[0], &right[0])
            }
            _ => unreachable!(),
        }
    };

    // The lowest bit in the index is ignored. It can point to either leaves
    x_index >>= 1;
    for path_i in path.iter() {
        hash = if x_index & 1 == 0 {
            hash_two_digests(&hash, path_i)
        } else {
            hash_two_digests(path_i, &hash)
        };
        x_index >>= 1;
    }
    assert_eq!(&hash, root);
}
