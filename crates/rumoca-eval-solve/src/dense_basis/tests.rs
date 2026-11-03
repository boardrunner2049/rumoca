use super::*;

#[test]
fn singular_trial_correction_is_minimum_norm_but_cannot_certify_a_basis() {
    let matrix = DenseStageMatrix::new(2, 3, &[1., 1., 0., 2., 2., 0.]).unwrap();
    let step = matrix.correction(&[2., 4.]).unwrap();
    assert!((step[0] + 1.).abs() < 1e-13);
    assert!((step[1] + 1.).abs() < 1e-13);
    assert_eq!(step[2], 0.);
    assert_eq!(
        matrix.independent_columns(&[ColumnChoice::Eligible(0); 3]),
        Err(DenseBasisError::Rank)
    );
}

#[test]
fn required_states_and_preference_groups_precede_numerical_pivoting() {
    let matrix = DenseStageMatrix::new(2, 4, &[1., 100., 1., 0., 0., 0., 0., 1.]).unwrap();
    let choices = [
        ColumnChoice::Eligible(0),
        ColumnChoice::Independent,
        ColumnChoice::Eligible(1),
        ColumnChoice::Dependent,
    ];
    assert_eq!(matrix.independent_columns(&choices).unwrap(), [1, 2]);
    assert_eq!(
        matrix.independent_columns(&[ColumnChoice::Independent; 4]),
        Err(DenseBasisError::Rank)
    );
}

#[test]
fn dependent_columns_must_be_independent_and_all_inputs_finite() {
    let matrix = DenseStageMatrix::new(2, 3, &[1., 2., 0., 0., 0., 1.]).unwrap();
    assert_eq!(
        matrix.independent_columns(&[
            ColumnChoice::Dependent,
            ColumnChoice::Dependent,
            ColumnChoice::Eligible(0)
        ]),
        Err(DenseBasisError::Rank)
    );
    assert!(matches!(
        DenseStageMatrix::new(1, 1, &[f64::NAN]),
        Err(DenseBasisError::NonFinite)
    ));
    assert_eq!(
        matrix.correction(&[0., f64::INFINITY]),
        Err(DenseBasisError::NonFinite)
    );
    assert!(matches!(
        DenseStageMatrix::new(usize::MAX, 2, &[]),
        Err(DenseBasisError::Shape)
    ));
}

/// The nalgebra SVD pseudo-inverse step the faer thin SVD replaced, with the
/// same relative truncation.
fn nalgebra_reference(matrix: &DenseStageMatrix, residual: &[f64]) -> DVector<f64> {
    let svd = nalgebra::linalg::SVD::try_new(matrix.0.clone(), true, true, f64::EPSILON, 4096)
        .expect("reference SVD converges");
    let threshold = matrix.threshold(svd.singular_values.amax());
    svd.solve(&(-DVector::from_column_slice(residual)), threshold)
        .expect("reference solve")
}

/// A deterministic dense stage with a rank deficiency: its last two rows
/// are combinations of the others.
fn deficient_stage() -> (DenseStageMatrix, Vec<f64>) {
    let (rows, columns) = (7, 9);
    let mut values = vec![0.0; rows * columns];
    for r in 0..rows - 2 {
        for c in 0..columns {
            values[r * columns + c] = ((r * 7 + c * 3) % 11) as f64 - 5.0 + 0.25 * (r + c) as f64;
        }
    }
    for c in 0..columns {
        values[5 * columns + c] = values[c] - 2.0 * values[columns + c];
        values[6 * columns + c] = 0.5 * values[2 * columns + c] + values[4 * columns + c];
    }
    let residual = (0..rows).map(|r| (r as f64 - 3.0) * 0.7).collect();
    (
        DenseStageMatrix::new(rows, columns, &values).unwrap(),
        residual,
    )
}

#[test]
fn thin_svd_correction_matches_the_nalgebra_pseudo_inverse_and_repeats_exactly() {
    let fixtures = [
        (
            DenseStageMatrix::new(2, 3, &[1., 1., 0., 2., 2., 0.]).unwrap(),
            vec![2., 4.],
        ),
        (
            DenseStageMatrix::new(2, 4, &[1., 100., 1., 0., 0., 0., 0., 1.]).unwrap(),
            vec![1., -3.],
        ),
        (
            DenseStageMatrix::new(2, 3, &[1., 2., 0., 0., 0., 1.]).unwrap(),
            vec![0.5, 2.],
        ),
        deficient_stage(),
    ];
    for (matrix, residual) in &fixtures {
        let step = matrix.correction(residual).unwrap();
        let reference = nalgebra_reference(matrix, residual);
        // Both are the minimum-norm least-squares solution on the same
        // numerical range; they agree to roundoff scaled by the step.
        let scale = reference.amax().max(1.0);
        for (value, expected) in step.iter().zip(reference.iter()) {
            assert!(
                (value - expected).abs() <= 1e-10 * scale,
                "{step:?} vs {reference:?}"
            );
        }
        assert_eq!(
            step,
            matrix.correction(residual).unwrap(),
            "run-to-run determinism"
        );
    }
}
