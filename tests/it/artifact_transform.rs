#[test]
#[allow(clippy::float_cmp)] // Integer inputs and matrices produce exactly representable integers.
fn component_then_build_transform() {
    let point = crate::artifact::transform([1., 2., 3.], Some("0 1 0 -1 0 0 0 0 1 10 20 30"));
    assert_eq!(point, [8., 21., 33.]);
    assert_eq!(
        crate::artifact::transform(point, Some("2 0 0 0 3 0 0 0 4 1 2 3")),
        [17., 65., 135.]
    );
    assert_eq!(crate::artifact::transform(point, None), point);
}
