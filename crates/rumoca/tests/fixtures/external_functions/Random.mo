// The MSL 4.0.0 `Modelica.Math.Random` structure in miniature: generators
// whose `random` is the ModelicaExternalC C function, initial states built
// by a loop over affine slices (`state[i-2:i-1]`), and a state vector sized
// by a package constant several packages deep. Expected values in the tests
// are those of the MSL C source (ModelicaRandom.c) itself.
package Rng
  constant Integer version = 1;
  package Math
    package Random
      package Generators
        package Xorshift64star
          constant Integer nState = 2;
          function initialState
            input Integer localSeed;
            input Integer globalSeed;
            output Integer state[nState];
          protected
            Real r;
          algorithm
            if localSeed == 0 and globalSeed == 0 then
              state := {126247697, globalSeed};
            else
              state := {localSeed, globalSeed};
            end if;
            for i in 1:10 loop
              (r, state) := random(state);
            end for;
          end initialState;
          pure function random
            input Integer stateIn[nState];
            output Real result;
            output Integer stateOut[nState];
            external "C" ModelicaRandom_xorshift64star(stateIn, stateOut, result);
          end random;
        end Xorshift64star;
        package Xorshift128plus
          constant Integer nState = 4;
          function initialState
            input Integer localSeed;
            input Integer globalSeed;
            output Integer state[nState];
          algorithm
            state := Utilities.initialStateWithXorshift64star(localSeed, globalSeed, size(state, 1));
          end initialState;
          pure function random
            input Integer stateIn[nState];
            output Real result;
            output Integer stateOut[nState];
            external "C" ModelicaRandom_xorshift128plus(stateIn, stateOut, result);
          end random;
        end Xorshift128plus;
      end Generators;
      package Utilities
        function initialStateWithXorshift64star
          import Rng.Math.Random.Generators.Xorshift64star;
          input Integer localSeed;
          input Integer globalSeed;
          input Integer nState(min = 1);
          output Integer[nState] state;
        protected
          Real r;
          Integer aux[2];
          Integer nStateEven;
        algorithm
          aux := Xorshift64star.initialState(localSeed, globalSeed);
          if nState >= 2 then
            state[1:2] := aux;
          else
            state[1] := aux[1];
          end if;
          nStateEven := 2*div(nState, 2);
          for i in 3:2:nStateEven loop
            (r, aux) := Xorshift64star.random(state[i-2:i-1]);
            state[i:i+1] := aux;
          end for;
          if nState >= 3 and nState <> nStateEven then
            (r, aux) := Xorshift64star.random(state[nState-2:nState-1]);
            state[nState] := aux[1];
          end if;
        end initialStateWithXorshift64star;
      end Utilities;
    end Random;
  end Math;
end Rng;

model RngTicks
  import Rng.Math.Random.Generators.Xorshift128plus;
  parameter Real period = 0.1;
  parameter Integer v = Rng.version;
  discrete Real tNext(start = period, fixed = true);
  discrete Integer s[Rng.Math.Random.Generators.Xorshift128plus.nState](
    start = Xorshift128plus.initialState(1, 2), each fixed = true);
  discrete Real r(start = 0, fixed = true);
  Real x(start = 0, fixed = true);
equation
  der(x) = r - x;
  when time >= pre(tNext) then
    tNext = pre(tNext) + period;
    (r, s) = Xorshift128plus.random(pre(s));
  end when;
end RngTicks;

model AtanEvents "An external builtin math function feeding a relation"
  function atan2b
    input Real u1;
    input Real u2;
    output Real y;
    external "builtin" y = atan2(u1, u2);
  end atan2b;
  Real x(start = 0, fixed = true);
  Real angle;
  Boolean past;
  Integer crossings(start = 0, fixed = true);
equation
  der(x) = 1;
  angle = atan2b(x, 1);
  past = angle > 0.5;
  when past then
    crossings = pre(crossings) + 1;
  end when;
end AtanEvents;

model UserExternal "A C external with no shipped source"
  function myGain
    input Real u;
    output Real y;
    external "C" y = my_gain(u) annotation(Library = "mygain");
  end myGain;
  Real x(start = 1, fixed = true);
equation
  der(x) = -myGain(x);
end UserExternal;
