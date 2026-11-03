type Mode = enumeration(off, low, high);
type PositiveReal = Real(min = 1, start = 5);

block RangedInputs
  input Real a(min = 1, max = 2);
  input Real c(max = -1);
  input Real r[2](each min = 1, each max = 4);
  input PositiveReal p;
  input Integer ki(min = 3);
  input Mode m;
  input Mode mh(start = Mode.high);
  output Real x(start = 0, fixed = true);
equation
  der(x) = a + c + p + ki;
end RangedInputs;
