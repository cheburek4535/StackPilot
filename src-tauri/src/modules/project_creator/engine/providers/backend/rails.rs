use crate::modules::project_creator::engine::preflight;
use crate::modules::project_creator::engine::providers::{write_file, RecipeProvider};
use crate::modules::project_creator::models::{Step, WizardContext};

pub struct RailsProvider;

impl RecipeProvider for RailsProvider {
    fn id(&self) -> &'static str {
        "rails"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        project_name: &str,
        _context: &WizardContext,
        segment: Option<&str>,
    ) -> Vec<Step> {
        let gemfile_path = match segment {
            Some(dir) => format!("{}/Gemfile", dir),
            None => "Gemfile".to_string(),
        };

        vec![
            write_file(
                "rails_gemfile",
                "Create Gemfile",
                "Gemfile",
                r#"# frozen_string_literal: true

source "https://rubygems.org"
git_source(:github) { |repo| "https://github.com/#{repo}.git" }

ruby ">= 3.0.0"

gem "rails", "~> 7.1.0"
gem "puma", ">= 5.0"
gem "sqlite3", ">= 1.4"
gem "bootsnap", require: false

group :development, :test do
  gem "debug", platforms: %i[ mri windows ]
end

group :development do
  gem "web-console"
end
"#,
            ),
            write_file(
                "rails_config_ru",
                "Create config.ru",
                "config.ru",
                r#"# frozen_string_literal: true

require_relative "config/environment"

run Rails.application
Rails.application.load_server
"#,
            ),
            write_file(
                "rails_rakefile",
                "Create Rakefile",
                "Rakefile",
                r#"# frozen_string_literal: true

require_relative "config/application"

Rails.application.load_tasks
"#,
            ),
            write_file(
                "rails_boot",
                "Create config/boot.rb",
                "config/boot.rb",
                r#"# frozen_string_literal: true

ENV["BUNDLE_GEMFILE"] ||= File.expand_path("../Gemfile", __dir__)

require "bundler/setup"
"#,
            ),
            write_file(
                "rails_application",
                "Create config/application.rb",
                "config/application.rb",
                &format!(
                    r#"# frozen_string_literal: true

require_relative "boot"

require "rails"
require "active_model/railtie"
require "active_job/railtie"
require "active_record/railtie"
require "action_controller/railtie"
require "action_view/railtie"
require "rails/test_unit/railtie"

Bundler.require(*Rails.groups)

module App
  class Application < Rails::Application
    config.load_defaults 7.1
    config.api_only = true
  end
end
"#
                ),
            ),
            write_file(
                "rails_environment",
                "Create config/environment.rb",
                "config/environment.rb",
                r#"# frozen_string_literal: true

require_relative "application"

Rails.application.initialize!
"#,
            ),
            write_file(
                "rails_routes",
                "Create config/routes.rb",
                "config/routes.rb",
                r#"# frozen_string_literal: true

Rails.application.routes.draw do
  get "up" => proc { [200, {}, ["OK"]] }
  root "application#index"
end
"#,
            ),
            write_file(
                "rails_database_yml",
                "Create config/database.yml",
                "config/database.yml",
                r#"default: &default
  adapter: sqlite3
  pool: <%= ENV.fetch("RAILS_MAX_THREADS") { 5 } %>
  timeout: 5000

development:
  <<: *default
  database: storage/development.sqlite3

test:
  <<: *default
  database: storage/test.sqlite3

production:
  <<: *default
  database: storage/production.sqlite3
"#,
            ),
            write_file(
                "rails_env_development",
                "Create config/environments/development.rb",
                "config/environments/development.rb",
                r#"# frozen_string_literal: true

require "active_support/core_ext/integer/time"

Rails.application.configure do
  config.enable_reloading = true
  config.eager_load = false
  config.consider_all_requests_local = true
  config.server_timing = true
end
"#,
            ),
            write_file(
                "rails_env_production",
                "Create config/environments/production.rb",
                "config/environments/production.rb",
                r#"# frozen_string_literal: true

require "active_support/core_ext/integer/time"

Rails.application.configure do
  config.enable_reloading = false
  config.eager_load = true
  config.consider_all_requests_local = false
end
"#,
            ),
            write_file(
                "rails_env_test",
                "Create config/environments/test.rb",
                "config/environments/test.rb",
                r#"# frozen_string_literal: true

require "active_support/core_ext/integer/time"

Rails.application.configure do
  config.enable_reloading = false
  config.eager_load = ENV["CI"].present?
  config.consider_all_requests_local = true
end
"#,
            ),
            write_file(
                "rails_controller",
                "Create app/controllers/application_controller.rb",
                "app/controllers/application_controller.rb",
                &format!(
                    r#"# frozen_string_literal: true

class ApplicationController < ActionController::API
  def index
    render json: {{
      message: "Hello from {} (Ruby on Rails)!",
      status: "online"
    }}
  end
end
"#,
                    project_name
                ),
            ),
            write_file(
                "rails_bin",
                "Create bin/rails",
                "bin/rails",
                r#"#!/usr/bin/env ruby
APP_PATH = File.expand_path("../config/application", __dir__)
require_relative "../config/boot"
require "rails/commands"
"#,
            ),
            preflight::manifest_check_step(
                "rails_gemfile_check",
                "Validate Rails Gemfile",
                &gemfile_path,
                "gemfile",
                &["rails"],
            ),
        ]
    }
}
