use aimer_shape::{
    DashSettings, FillRule, FillStyle, Point, ShapeColor, ShapeError, ShapeFit, ShapeHitTest,
    ShapeLimits, ShapePathBuilder, ShapeSize, StrokeStyle,
};

#[test]
fn builder_keeps_commands_bounds_and_arc_convenience_deterministic() {
    let path = ShapePathBuilder::new()
        .move_to(0.0, 0.0)
        .quadratic_to(5.0, 10.0, 10.0, 0.0)
        .cubic_to(12.0, -4.0, 18.0, -4.0, 20.0, 0.0)
        .close()
        .build()
        .expect("finite path");

    assert_eq!(path.command_count(), 4);
    assert_eq!(path.contour_count(), 1);
    assert!(path.bounds().min.y < 0.0);
    assert!(path.bounds().max.y > 0.0);
    assert_eq!(path.encode(), path.encode());
    let mut encoded_hash = 0xcbf29ce484222325u64;
    for byte in path.encode() {
        encoded_hash ^= u64::from(byte);
        encoded_hash = encoded_hash.wrapping_mul(0x100000001b3);
    }
    assert_eq!(path.id(), aimer_shape::ShapePathId(encoded_hash));

    let ellipse = ShapePathBuilder::new()
        .ellipse(20.0, 30.0, 8.0, 4.0, 0.25)
        .build()
        .expect("finite ellipse");
    assert!(ellipse.bounds().contains(Point::new(20.0, 30.0)));
    assert_eq!(ellipse, ellipse.clone());

    let positive_zero = ShapePathBuilder::new()
        .move_to(0.0, 0.0)
        .line_to(1.0, 0.0)
        .line_to(1.0, 1.0)
        .close()
        .build()
        .unwrap();
    let negative_zero = ShapePathBuilder::new()
        .move_to(-0.0, -0.0)
        .line_to(1.0, -0.0)
        .line_to(1.0, 1.0)
        .close()
        .build()
        .unwrap();
    assert_eq!(positive_zero.encode(), negative_zero.encode());
    assert_eq!(positive_zero.id(), negative_zero.id());
}

#[test]
fn builder_rejects_malformed_non_finite_zero_length_and_excessive_paths() {
    assert!(matches!(
        ShapePathBuilder::new().line_to(1.0, 1.0).build(),
        Err(ShapeError::CommandBeforeMove { .. })
    ));
    assert!(matches!(
        ShapePathBuilder::new().move_to(f32::NAN, 0.0).build(),
        Err(ShapeError::NonFinite { .. })
    ));
    assert!(matches!(
        ShapePathBuilder::new()
            .move_to(1.0, 1.0)
            .line_to(1.0, 1.0)
            .build(),
        Err(ShapeError::ZeroLengthSegment { .. })
    ));
    assert!(matches!(
        ShapePathBuilder::new().move_to(0.0, 0.0).close().build(),
        Err(ShapeError::EmptyContour { .. })
    ));

    let limits = ShapeLimits {
        max_commands: 2,
        ..ShapeLimits::default()
    };
    assert!(matches!(
        ShapePathBuilder::with_limits(limits)
            .move_to(0.0, 0.0)
            .line_to(1.0, 0.0)
            .line_to(1.0, 1.0)
            .build(),
        Err(ShapeError::TooManyCommands { limit: 2 })
    ));
    assert!(matches!(
        aimer_shape::ShapePath::try_from_commands_with_limits(
            [
                aimer_shape::ShapeCommand::move_to(0.0, 0.0),
                aimer_shape::ShapeCommand::line_to(1.0, 0.0),
                aimer_shape::ShapeCommand::line_to(1.0, 1.0),
            ],
            limits,
        ),
        Err(ShapeError::TooManyCommands { limit: 2 })
    ));
    assert!(matches!(
        ShapeLimits {
            max_commands: aimer_shape::DEFAULT_MAX_COMMANDS + 1,
            ..ShapeLimits::default()
        }
        .validate(),
        Err(ShapeError::InvalidLimit("max_commands"))
    ));
    let arc_limits = ShapeLimits {
        max_abs_coordinate: 5.0,
        ..ShapeLimits::default()
    };
    assert!(matches!(
        aimer_shape::ShapePath::try_from_commands_with_limits(
            [
                aimer_shape::ShapeCommand::move_to(0.0, 0.0),
                aimer_shape::ShapeCommand::arc_to(
                    3.0,
                    0.0,
                    3.0,
                    3.0,
                    std::f32::consts::PI,
                    std::f32::consts::TAU,
                    0.0,
                ),
            ],
            arc_limits,
        ),
        Err(ShapeError::CoordinateOutOfRange {
            field: "extent",
            ..
        })
    ));
}

#[test]
fn paint_validation_hit_testing_and_fit_are_bounded() {
    let square = ShapePathBuilder::new()
        .move_to(0.0, 0.0)
        .line_to(10.0, 0.0)
        .line_to(10.0, 10.0)
        .line_to(0.0, 10.0)
        .close()
        .build()
        .unwrap();
    let fill = FillStyle::solid(ShapeColor::rgba(0.2, 0.4, 0.8, 1.0));
    assert!(square.hit_test(Point::new(5.0, 5.0), ShapeHitTest::Fill, Some(&fill), None));
    assert!(!square.hit_test(Point::new(15.0, 5.0), ShapeHitTest::Fill, Some(&fill), None));

    let stroke = StrokeStyle::new(2.0, ShapeColor::BLACK)
        .unwrap()
        .with_dash([3.0, 2.0], 0.5)
        .unwrap();
    assert!(square.hit_test(
        Point::new(5.0, 0.0),
        ShapeHitTest::Stroke,
        None,
        Some(&stroke)
    ));
    assert!(StrokeStyle::new(f32::NAN, ShapeColor::BLACK).is_err());
    assert!(
        StrokeStyle::new(1.0, ShapeColor::BLACK)
            .unwrap()
            .with_dash([1.0, 0.0], 0.0)
            .is_err()
    );
    assert!(matches!(
        DashSettings::with_limit([], 0.0, aimer_shape::DEFAULT_MAX_DASH_SEGMENTS + 1),
        Err(aimer_shape::PaintError::InvalidDashLimit { .. })
    ));

    let transform = ShapeFit::Contain
        .transform(square.bounds(), ShapeSize::new(20.0, 10.0))
        .unwrap();
    assert_eq!(transform.sx, 1.0);
    assert_eq!(transform.sy, 1.0);
    assert_eq!(transform.tx, 5.0);
    assert_eq!(transform.ty, 0.0);
    assert_eq!(FillRule::EvenOdd, FillRule::EvenOdd);
}

#[test]
fn none_fit_preserves_the_path_local_coordinates() {
    let path = ShapePathBuilder::new()
        .move_to(12.0, 8.0)
        .line_to(32.0, 8.0)
        .line_to(32.0, 18.0)
        .line_to(12.0, 18.0)
        .close()
        .build()
        .unwrap();

    let transform = ShapeFit::None
        .transform(path.bounds(), ShapeSize::new(100.0, 80.0))
        .unwrap();

    assert_eq!(transform, aimer_shape::ShapeTransform::identity());
}

#[test]
fn fill_rules_distinguish_a_nested_contour() {
    let nested = ShapePathBuilder::new()
        .move_to(0.0, 0.0)
        .line_to(20.0, 0.0)
        .line_to(20.0, 20.0)
        .line_to(0.0, 20.0)
        .close()
        .move_to(5.0, 5.0)
        .line_to(15.0, 5.0)
        .line_to(15.0, 15.0)
        .line_to(5.0, 15.0)
        .close()
        .build()
        .unwrap();
    let color = ShapeColor::BLACK;
    let even_odd = FillStyle::new(color, FillRule::EvenOdd);
    let non_zero = FillStyle::new(color, FillRule::NonZero);

    assert!(!nested.hit_test(
        Point::new(10.0, 10.0),
        ShapeHitTest::Fill,
        Some(&even_odd),
        None,
    ));
    assert!(nested.hit_test(
        Point::new(10.0, 10.0),
        ShapeHitTest::Fill,
        Some(&non_zero),
        None,
    ));
    assert!(nested.hit_test(
        Point::new(2.0, 2.0),
        ShapeHitTest::Fill,
        Some(&even_odd),
        None,
    ));
}
