pipeline {
    agent any
    stages {
        stage('build') {
            steps {
                sh 'cargo build --release'
            }
        }
        stage('test') {
            steps {
                sh 'cargo test'
            }
        }
    }
}
