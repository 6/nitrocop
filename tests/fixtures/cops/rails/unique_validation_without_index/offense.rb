class User < ApplicationRecord
  validates :account, uniqueness: true
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/UniqueValidationWithoutIndex: Uniqueness validation should have a unique index on the database column.

  validates :username, uniqueness: { case_sensitive: false }
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/UniqueValidationWithoutIndex: Uniqueness validation should have a unique index on the database column.

  validates :account, uniqueness: { scope: :organization_id }
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/UniqueValidationWithoutIndex: Uniqueness validation should have a unique index on the database column.
end

class ServiceStatus < ActiveRecord::Base
  validates :name, :presence => true
  validates :permalink, :presence => true, :uniqueness => true
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/UniqueValidationWithoutIndex: Uniqueness validation should have a unique index on the database column.
end

module Budget
  class ContentBlock < ApplicationRecord
    validates :locale, presence: true
    validates :heading, presence: true, uniqueness: { scope: :locale }
    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/UniqueValidationWithoutIndex: Uniqueness validation should have a unique index on the database column.

    belongs_to :heading
  end
end

module Wkbk
  class Book < ApplicationRecord
    with_options allow_blank: true do
      validates :title, uniqueness: { scope: :user_id, case_sensitive: true, message: "が重複しています" }
      ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/UniqueValidationWithoutIndex: Uniqueness validation should have a unique index on the database column.
    end
  end
end

class DnsAlias < ApplicationRecord
  validates :name, presence: true, uniqueness: true, format: { with: /\A[a-z][a-z0-9-]*\z/i }
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Rails/UniqueValidationWithoutIndex: Uniqueness validation should have a unique index on the database column.
end
