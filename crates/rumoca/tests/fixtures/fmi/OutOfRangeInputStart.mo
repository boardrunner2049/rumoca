// A start written outside the input's declared range is a model error: FMI
// requires min <= start <= max, and the exporter does not replace a start the
// model states with one it never states.
block OutOfRangeInputStart
  input Real a(start = 5, max = 2);
  output Real y;
equation
  y = a;
end OutOfRangeInputStart;
