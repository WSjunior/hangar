//! Mapa com teto de itens para os caches da lista: a invalidação por mtime diz quando reler, o
//! teto diz quanto guardar. Cheio, a chave nova tira a usada há mais tempo.
use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::Hash;

/// Teto dos caches por sessão ou por arquivo de sessão: folga larga sobre as sessões vivas.
pub const SESSION_CAP: usize = 256;

pub struct Capped<K, V> {
    cap: usize,
    clock: u64,
    map: HashMap<K, (u64, V)>,
}

impl<K: Hash + Eq + Clone, V> Capped<K, V> {
    pub fn new(cap: usize) -> Self { Self { cap: cap.max(1), clock: 0, map: HashMap::new() } }

    pub fn get<Q: Hash + Eq + ?Sized>(&mut self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
    {
        self.clock += 1;
        let clock = self.clock;
        self.map.get_mut(key).map(|e| {
            e.0 = clock;
            &e.1
        })
    }

    /// Sem contar como uso: para quem só consulta.
    pub fn peek<Q: Hash + Eq + ?Sized>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
    {
        self.map.get(key).map(|e| &e.1)
    }

    pub fn insert(&mut self, key: K, value: V) {
        self.clock += 1;
        if self.map.len() >= self.cap && !self.map.contains_key(&key) {
            // ponytail: varredura O(n) só com o mapa cheio; n é o teto, centenas.
            if let Some(old) = self.map.iter().min_by_key(|(_, e)| e.0).map(|(k, _)| k.clone()) {
                self.map.remove(&old);
            }
        }
        self.map.insert(key, (self.clock, value));
    }

    pub fn remove<Q: Hash + Eq + ?Sized>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
    {
        self.map.remove(key).map(|e| e.1)
    }

    pub fn len(&self) -> usize { self.map.len() }

    pub fn is_empty(&self) -> bool { self.map.is_empty() }
}

impl<K: Hash + Eq + Clone, V> Default for Capped<K, V> {
    fn default() -> Self { Self::new(SESSION_CAP) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_map_drops_least_recently_used() {
        let mut m = Capped::new(2);
        m.insert("a", 1);
        m.insert("b", 2);
        assert_eq!(m.get("a"), Some(&1));
        m.insert("c", 3);
        assert_eq!((m.len(), m.peek("b"), m.peek("a"), m.peek("c")), (2, None, Some(&1), Some(&3)));
        m.insert("a", 9);
        assert_eq!((m.len(), m.peek("a")), (2, Some(&9)), "chave existente não tira ninguém");
    }
}
