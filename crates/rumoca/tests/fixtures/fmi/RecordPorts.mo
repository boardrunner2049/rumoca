// A top-level input and output of record type: the prefix applies to every
// field (MLS 4.4.2.2), so each field is an interface input or output. The
// same record output of a nested block stays local.
record Pair
  Real x;
  Real y[2];
end Pair;

block Sub
  input Real v;
  output Pair q;
equation
  q.x = v;
  q.y = {v, 2 * v};
end Sub;

block RecordPorts
  input Pair u;
  output Pair r;
  output Real z;
  Sub sub(v = u.x);
equation
  r.x = 2 * u.x;
  r.y = u.y;
  z = sub.q.x + sub.q.y[2];
end RecordPorts;
