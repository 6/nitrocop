task :foo do
^^^^ Rake/Desc: Describe the task with `desc` method.
  puts 'foo'
end

task :bar
^^^^ Rake/Desc: Describe the task with `desc` method.

task :baz do
^^^^ Rake/Desc: Describe the task with `desc` method.
  puts 'baz'
end

begin
  require 'foo'
rescue LoadError
  task :gem do
  ^^^^ Rake/Desc: Describe the task with `desc` method.
    abort 'not available'
  end
  task :package do
  ^^^^ Rake/Desc: Describe the task with `desc` method.
    abort 'not available'
  end
end

if HAVE_RCOV
  RCov::VerifyTask.new(:verify) do |t|
    t.threshold = 1
  end
  task :verify => :rcov
  ^^^^ Rake/Desc: Describe the task with `desc` method.
  remove_desc_for_task %w(spec:clobber_rcov)
end

def deprecated_task(name, new_name)
  warn name
  task name => new_name do
  ^^^^ Rake/Desc: Describe the task with `desc` method.
    warn "deprecated #{name}"
  end
end
