pipeline {
    agent any
    
    stages {
        stage("Deploy") {
            matrix {
                axes {
                    axis {
                        name "DEPLOY_ENVIRONMENT"
                        values "DEPLOY_DEV", "DEPLOY_STAGING", "DEPLOY_PROD"
                    }
                }
                stages {
                    stage("Deploy") {
                        when {
                            beforeAgent true
                            expression {
                                params[DEPLOY_ENVIRONMENT] == true
                            }
                        }
                        steps {
                            echo "Deploying to $DEPLOY_ENVIRONMENT"
                        }
                    }
                }
            }
        }
    }
}