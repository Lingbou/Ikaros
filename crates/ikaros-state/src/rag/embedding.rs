// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::Result;
use std::{
    collections::{BTreeSet, HashMap},
    hash::{Hash, Hasher},
};

pub trait EmbeddingProvider: Send + Sync {
    fn name(&self) -> &str;
    fn embed(&self, text: &str) -> Result<Vec<f32>>;
}

#[derive(Debug, Clone, Default)]
pub struct MockEmbeddingProvider;

impl EmbeddingProvider for MockEmbeddingProvider {
    fn name(&self) -> &str {
        "mock"
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        if tokenize(text).is_empty() {
            Ok(vec![0.0])
        } else {
            Ok(vec![1.0])
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct HashEmbeddingProvider;

impl EmbeddingProvider for HashEmbeddingProvider {
    fn name(&self) -> &str {
        "hash"
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut buckets = vec![0.0; 32];
        for token in tokenize(text) {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            token.hash(&mut hasher);
            let idx = (hasher.finish() as usize) % buckets.len();
            buckets[idx] += 1.0;
        }
        Ok(buckets)
    }
}

#[derive(Debug, Clone, Default)]
pub struct SparseEmbeddingProvider;

impl EmbeddingProvider for SparseEmbeddingProvider {
    fn name(&self) -> &str {
        "sparse"
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut buckets = vec![0.0; 64];
        for token in tokenize(text) {
            let idx = sparse_token_index(&token) % buckets.len();
            buckets[idx] += 1.0;
        }
        normalize_vector(&mut buckets);
        Ok(buckets)
    }
}

pub(crate) fn tokenize(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_alphanumeric() && ch != '_')
        .map(str::trim)
        .filter(|token| token.len() > 1)
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

pub(crate) fn query_tokens(text: &str) -> BTreeSet<String> {
    tokenize(text).into_iter().collect()
}

pub(crate) fn lexical_score(query_tokens: &BTreeSet<String>, content: &str) -> f32 {
    let mut counts = HashMap::<String, usize>::new();
    for token in tokenize(content) {
        *counts.entry(token).or_default() += 1;
    }
    query_tokens
        .iter()
        .map(|token| counts.get(token).copied().unwrap_or_default() as f32)
        .sum()
}

pub(crate) fn combined_score(lexical: f32, vector: f32) -> f32 {
    lexical + vector
}

pub(crate) fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let len = left.len().min(right.len());
    let mut dot = 0.0;
    let mut left_norm = 0.0;
    let mut right_norm = 0.0;
    for idx in 0..len {
        dot += left[idx] * right[idx];
        left_norm += left[idx] * left[idx];
        right_norm += right[idx] * right[idx];
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        dot / (left_norm.sqrt() * right_norm.sqrt())
    }
}

fn normalize_vector(vector: &mut [f32]) {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in vector {
            *value /= norm;
        }
    }
}

fn sparse_token_index(token: &str) -> usize {
    token.bytes().fold(0usize, |acc, byte| {
        acc.wrapping_mul(31).wrapping_add(byte as usize)
    })
}
