# nitrocop-config: EnforcedStyle: where

User.where(active: true).exists?
User.where(['name = ?', 'john']).exists?
User.where('name = ?', 'john').exists?
User.where("length(name) > 10").exists?
user.posts.where(published: true).exists?
User.exists?
User.exists?('name = ?', 'john')
User.exists?(*conditions)
