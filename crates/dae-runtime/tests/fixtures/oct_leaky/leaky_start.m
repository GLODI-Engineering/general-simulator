% A struct-valued octblock state -- see leaky.m. Used by tests/checkpoint_resume.rs.
function state = leaky_start()
  state = struct('sum', 0, 'n', int32(0));
end
