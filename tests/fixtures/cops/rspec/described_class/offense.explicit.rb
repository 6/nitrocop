RSpec.describe Alchemy::Configuration do
  let(:configuration) do
    Class.new(described_class).new
              ^^^^^^^^^^^^^^^ RSpec/DescribedClass: Use `Alchemy::Configuration` instead of `described_class`.
  end
end
