use super::*;

/// A constant cube makes the prepared pixels independent of camera angle.
fn faces(color: [u8; 4]) -> PanoramaFaces {
    PanoramaFaces::new(1, std::array::from_fn(|_| color.to_vec())).unwrap()
}

#[test]
fn panorama_banner_choice_is_stable_for_each_account() {
    assert_eq!(profile_key(""), VIEWS[0].key);
    assert_eq!(profile_key("123"), VIEWS[6].key);
    assert_eq!(
        profile_key("😀"),
        VIEWS[(0xd83d + 0xde00) % PROFILE_BANNER_COUNT].key
    );
    for index in 0..PROFILE_BANNER_COUNT {
        let xuid = index.to_string();
        assert_eq!(profile_key(&xuid), VIEWS[index].key);
        assert_eq!(profile_key(&xuid), profile_key(&xuid));
    }
}

#[test]
fn panorama_missing_faces_keep_plain_slots_and_do_not_change_account_selection() {
    let mut images = OreUiImages::default();
    append(&mut images, None, None);
    assert!(images.pages.is_empty() && images.sprites.is_empty());
    let color = [70, 140, 210, 255];
    append(&mut images, Some(&faces(color)), None);
    assert_eq!(images.pages.len(), 1);
    for view in &VIEWS {
        assert_eq!(
            images.sprites.contains_key(view.key),
            view.set == PanoramaSet::Primary
        );
    }
    assert!(!images.sprites.contains_key(profile_key("123")));
    let sprite = images.sprites[CREATE_WORLD_KEY];
    let page = &images.pages[usize::from(sprite.page)];
    assert_eq!(sprite.bounds[2] - sprite.bounds[0], VIEW_SIZE[0] as u16);
    assert_eq!(sprite.bounds[3] - sprite.bounds[1], VIEW_SIZE[1] as u16);
    let offset = (usize::from(sprite.bounds[1]) * page.dimensions[0] as usize
        + usize::from(sprite.bounds[0]))
        * 4;
    assert_eq!(&page.pixels[offset..offset + 4], &color);
    assert!(page.pixels.len() <= render_model::MAX_UI_FIXED_TEXTURE_BYTES);
}

#[test]
fn panorama_texture_budget_rejection_preserves_existing_artwork() {
    let mut images = OreUiImages::default();
    images.pages.push(OreUiPage {
        dimensions: [1, 1],
        pixels: vec![0; render_model::MAX_UI_FIXED_TEXTURE_BYTES].into(),
    });
    append(&mut images, Some(&faces([255; 4])), None);
    assert_eq!(images.pages.len(), 1);
    assert!(images.sprites.is_empty());
}

#[test]
fn panorama_all_nine_views_share_one_fixed_page() {
    let mut images = OreUiImages::default();
    append(
        &mut images,
        Some(&faces([60, 120, 180, 255])),
        Some(&faces([180, 120, 60, 255])),
    );
    assert_eq!(images.pages.len(), 1);
    assert_eq!(images.sprites.len(), VIEWS.len());
    let page = &images.pages[0];
    assert!(page.pixels.len() < render_model::MAX_UI_FIXED_TEXTURE_BYTES);
    for view in &VIEWS {
        let sprite = images.sprites[view.key];
        let offset = (usize::from(sprite.bounds[1]) * page.dimensions[0] as usize
            + usize::from(sprite.bounds[0]))
            * 4;
        let expected = match view.set {
            PanoramaSet::Primary => [60, 120, 180, 255],
            PanoramaSet::Alternate => [180, 120, 60, 255],
        };
        assert_eq!(&page.pixels[offset..offset + 4], &expected);
    }
}
