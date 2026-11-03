// A top-level input and output of record type: the prefix applies to every
// field (MLS 4.4.2.2), so each field is an interface input or output. The
// same record output of a nested block stays local. A `parameter` field stays
// a parameter (an FMI output cannot be fixed or tunable). An input field (or a
// scalar input) with a declaration equation is still an input; it holds the
// equation's value until the importer writes it, so that value is its start.
record Pair
  Real x;
  Real y[2];
end Pair;

record Reading
  Real x;
  Real y[2];
  parameter Real k = 2;
end Reading;

record Bounded
  Real a;
  Real b = 7;
end Bounded;

block Sub
  input Real v;
  output Pair q;
equation
  q.x = v;
  q.y = {v, 2 * v};
end Sub;

block RecordPorts
  input Pair u;
  input Bounded w;
  input Real v = 3;
  output Reading r;
  output Real z;
  Sub sub(v = u.x);
equation
  r.x = r.k * u.x + w.a + w.b + v;
  r.y = u.y;
  z = sub.q.x + sub.q.y[2];
end RecordPorts;
