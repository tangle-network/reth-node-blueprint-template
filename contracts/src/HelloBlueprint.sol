// SPDX-License-Identifier: UNLICENSE
pragma solidity >=0.8.13;

import "tnt-core/BlueprintServiceManagerBase.sol";

/**
 * @title HelloBlueprint
 * @dev This contract is an example of a service blueprint that provides a single service.
 * @dev For all supported hooks, check the `BlueprintServiceManagerBase` contract.
 */
contract HelloBlueprint is BlueprintServiceManagerBase {
    /**
     * @dev Hook for service operator registration. Called when a service operator
     * attempts to register with the blueprint.
     * @param operator The operator's details.
     * @param _registrationInputs Inputs required for registration.
     */
    function onRegister(
        ServiceOperators.OperatorPreferences calldata operator,
        bytes calldata _registrationInputs
    )
        public
        payable
        override
        onlyFromMaster
    {
        // Do something with the operator's details
    }

    /**
     * @dev Hook for service instance requests. Called when a user requests a service
     * instance from the blueprint.
     * @param params The parameters for the service request.
     */
    function onRequest(ServiceOperators.RequestParams calldata params)
        public
        payable
        override
        onlyFromMaster
    {
        // Do something with the service request
    }

    /**
     * @dev Hook for handling job call results. Called when operators send the result
     * of a job execution.
     * @param serviceId The ID of the service related to the job.
     * @param job The job identifier.
     * @param _jobCallId The unique ID for the job call.
     * @param operator The operator sending the result in bytes format.
     * @param _inputs Inputs used for the job execution.
     * @param _outputs Outputs resulting from the job execution.
     */
    function onJobResult(
        uint64 serviceId,
        uint8 job,
        uint64 _jobCallId,
        ServiceOperators.OperatorPreferences calldata operator,
        bytes calldata _inputs,
        bytes calldata _outputs
    ) public payable virtual override onlyFromMaster {
        // Do something with the job call result
    }

    /**
     * @dev Converts a public key to an operator address.
     * @param publicKey The public key to convert.
     * @return operator address The operator address.
     */
    function operatorAddressFromPublicKey(bytes calldata publicKey) internal pure returns (address operator) {
        return address(uint160(uint256(keccak256(publicKey))));
    }
}
