type Mode = enumeration(off, low, high);

block UnstartedInputs
  input Real u;
  input Real w[2];
  input Integer k;
  input Boolean b;
  input Mode m;
  input Real s(start = 3);
  output Real x(start = 0, fixed = true);
  output Real y;
equation
  der(x) = 1 + u + w[1] + w[2] + k + (if b then 1 else 0);
  y = x + s;
end UnstartedInputs;
