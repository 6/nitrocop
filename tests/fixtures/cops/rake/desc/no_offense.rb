desc 'Do foo'
task :foo do
  puts 'foo'
end

desc 'Do bar'
task :bar

task :default

task default: [:foo]

begin
  require 'rubocop/rake_task'
  RuboCop::RakeTask.new :lint do |t|
    t.patterns = %w(lib/**/*.rb)
  end
rescue LoadError => e
  task :lint do
    raise 'Failed to load lint task.'
  end
end

begin
  require 'foo'
rescue LoadError
  puts 'oops'
ensure
  task :cleanup do
    puts 'clean'
  end
end

def deprecated_task(name, new_name)
  task name => new_name do
    warn "deprecated #{name}"
  end
end

if Rails.env.production? || Rails.env == 'fake_production'
  task :eslint do
    puts 'Skipping eslint checking in production.'
  end
end

task :clobber => 'spec:clobber_rcov' if HAVE_RCOV
