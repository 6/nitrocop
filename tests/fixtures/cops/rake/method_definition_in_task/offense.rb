task :foo do
  def helper
  ^^^^^^^^^^ Rake/MethodDefinitionInTask: Do not define a method in rake task, because it will be defined to the top level.
    puts 'help'
  end
end

namespace :ns do
  def another_helper
  ^^^^^^^^^^^^^^^^^^ Rake/MethodDefinitionInTask: Do not define a method in rake task, because it will be defined to the top level.
    puts 'help'
  end
end

task :baz do
  def yet_another
  ^^^^^^^^^^^^^^^ Rake/MethodDefinitionInTask: Do not define a method in rake task, because it will be defined to the top level.
  end
end

task :inline_class_new do
  task_stub = Class.new(Rake::TestTask) { def define; end }.new # no-op define
                                          ^^^^^^^^^^^^^^^ Rake/MethodDefinitionInTask: Do not define a method in rake task, because it will be defined to the top level.
end

task :inherited_class_new do
  target_attachment = Class.new(Attachment) do
    def self.store_all!(attachments)
    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rake/MethodDefinitionInTask: Do not define a method in rake task, because it will be defined to the top level.
    end

    def self.store!(attachment)
    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rake/MethodDefinitionInTask: Do not define a method in rake task, because it will be defined to the top level.
    end

    def self.to_s
    ^^^^^^^^^^^^^ Rake/MethodDefinitionInTask: Do not define a method in rake task, because it will be defined to the top level.
      "attachment"
    end
  end
end
