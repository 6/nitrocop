class User < ApplicationRecord
  # Has a matching unique index on email
  validates :email, uniqueness: true

  # Not a uniqueness validation
  validates :account, length: { minimum: 5 }

  # Uniqueness explicitly false
  validates :account, uniqueness: false

  # Presence validation
  validates :name, presence: true

  # Format validation
  validates :email, format: { with: /\A[^@]+@[^@]+\z/ }

  # Other validation types
  validates :account, numericality: { greater_than: 0 }

  # Conditional uniqueness — skipped
  validates :account, uniqueness: true, if: :active?

  # Conditional inside uniqueness hash — skipped
  validates :account, uniqueness: { unless: :draft? }
end

module Budget
  class IndexedContentBlock < ApplicationRecord
    validates :heading, uniqueness: { scope: :locale }

    belongs_to :heading
  end
end
