//! Source routes retained alongside the compiled world carrier.

use super::*;

/// Records terrain destinations for weighted leaves and their representative selectors.
pub(super) fn build(
    pack: &PackSources,
    descriptors: &BTreeMap<Descriptor, u32>,
    materials: &[Material],
    animations: &[Animation],
) -> MaterialKeys {
    let mut routes = Vec::new();
    for (descriptor, &id) in descriptors {
        let material = &materials[id as usize];
        let animated = animations
            .get(material.animation as usize)
            .map(|animation| (animation.atlas_index, animation.atlas_tile_variant));
        let (key, index, variant) = destination(descriptor, animated);
        if material.variation_count > 0 {
            routes.push((id, key, (index, 0)));
            routes.extend(
                (0..material.variation_count)
                    .map(|variant| (material.variation_start + variant, key, (index, variant))),
            );
        } else if !pack
            .terrain
            .position_variations
            .contains_key(&(descriptor.texture_key.clone(), descriptor.state_variant))
        {
            routes.push((id, key, (index, variant)));
        }
    }
    MaterialKeys::from_entries(routes.iter().map(|&(id, key, _)| (id, key)))
        .with_aliases(pack.terrain.first_paths())
        .with_flipbooks(&pack.flipbooks)
        .with_flipbook_destinations(pack.terrain.flipbook_destinations())
        .with_flipbook_selectors(routes.into_iter().map(|(id, _, selector)| (id, selector)))
}

/// Identifies a material's terrain destination independently of its deduplicated pixels.
pub(super) fn destination(
    descriptor: &Descriptor,
    animated: Option<(u32, u32)>,
) -> (&str, u32, u32) {
    let (index, variant) = animated.unwrap_or_else(|| {
        descriptor
            .flipbook_selector
            .unwrap_or((descriptor.state_variant, 0))
    });
    (&descriptor.texture_key, index, variant)
}

/// Adds generated materials without losing the original routes or animation descriptions.
pub(super) fn extend(
    keys: MaterialKeys,
    lily_pad_keys: &[(u32, Box<str>)],
    lily_pad_tints: impl IntoIterator<Item = (Box<str>, [u8; 3])>,
    seasonal_copies: &[(u32, u32)],
) -> MaterialKeys {
    let original = keys
        .keys()
        .flat_map(|key| keys.materials(key).iter().map(move |&id| (id, key)))
        .collect::<Vec<_>>();
    MaterialKeys::from_entries(
        original
            .iter()
            .copied()
            .chain(lily_pad_keys.iter().map(|(id, key)| (*id, key.as_ref())))
            .chain(seasonal_copies.iter().flat_map(|&(copy, original_id)| {
                original
                    .iter()
                    .filter_map(move |&(id, key)| (id == original_id).then_some((copy, key)))
            })),
    )
    .with_aliases(keys.aliases())
    .with_fixed_tints(lily_pad_tints)
    .with_flipbooks(keys.flipbooks())
    .with_flipbook_destinations(keys.flipbook_destinations())
    .with_flipbook_selectors(
        keys.flipbook_selectors().chain(
            seasonal_copies
                .iter()
                .map(|&(copy, original)| (copy, keys.flipbook_selector(original))),
        ),
    )
}

#[cfg(test)]
mod tests;
